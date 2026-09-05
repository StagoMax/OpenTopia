use super::{
    decode_typed_tool_input, enforce_policy_decision, enforce_read_policy,
    normalize_workspace_path, required_typed_string, ToolInvocationContext,
};
use crate::execution::FileReadRequest;
use crate::file_mutation::{read_optional, FileMutationBatch, PreparedFileMutation};
use crate::model::{ModelContentPart, ToolResult};
use crate::spreadsheet::{
    edit_workbook_structure, execute_spreadsheet, format_a1_address, transform_cell_input,
    transform_number_format, CellAddress, CellRange, CellUpdate, DelimitedFormat,
    DelimitedFormulaMode, EditWorkbookStructureRequest, ExportDelimitedRequest, FilterRowsRequest,
    FindCellsRequest, FormulaInput, InspectDelimitedRequest, InspectWorkbookRequest,
    ListSheetsRequest, ReadRangeRequest, ReadRangesRequest, SheetRangeRequest, SheetWriteRequest,
    SpreadsheetAction, SpreadsheetCell, SpreadsheetCellInput, SpreadsheetCellValue,
    SpreadsheetFileFormat, SpreadsheetFilterCondition, SpreadsheetFilterMatchMode,
    SpreadsheetFilterReturnMode, SpreadsheetRequest, SpreadsheetResult, SpreadsheetSheetValidation,
    SpreadsheetStructureOperation, SpreadsheetTextMatchMode, SpreadsheetValueTransform,
    ValidateWorkbookRequest, WriteWorkbookRequest, EXCEL_MAX_ROWS,
    MAX_INPUT_FILE_BYTES as MAX_SPREADSHEET_INPUT_BYTES,
};
use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SpreadsheetToolAction {
    InspectDelimited,
    Inspect,
    ListSheets,
    ReadRange,
    ReadRanges,
    ReadRows,
    ReadColumns,
    Find,
    FilterRows,
    Validate,
    ExportDelimited,
    Write,
    WriteRows,
    WriteColumns,
    CopyRanges,
    CopyRows,
    ConvertRanges,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum SpreadsheetCopyContentMode {
    #[default]
    Values,
    ValuesAndFormulas,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetRangeCopy {
    pub(super) source_path: String,
    pub(super) source_sheet: String,
    pub(super) source_start: CellAddress,
    #[schemars(range(min = 1))]
    pub(super) row_count: u32,
    #[schemars(range(min = 1))]
    pub(super) column_count: u32,
    pub(super) destination_sheet: String,
    pub(super) destination_start: CellAddress,
    #[serde(default)]
    pub(super) content_mode: SpreadsheetCopyContentMode,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetRangeConversion {
    pub(super) sheet: String,
    pub(super) range: CellRange,
    #[schemars(length(min = 1, max = 8))]
    pub(super) transforms: Vec<SpreadsheetValueTransform>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetColumnCopy {
    pub(super) source_column: u32,
    pub(super) destination_column: u32,
    #[serde(default)]
    #[schemars(length(max = 8))]
    pub(super) transforms: Vec<SpreadsheetValueTransform>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetRowCopy {
    pub(super) source_path: String,
    pub(super) source_sheet: String,
    pub(super) source_header_row: u32,
    pub(super) source_data_row: u32,
    pub(super) destination_sheet: String,
    pub(super) destination_header_row: u32,
    #[schemars(length(min = 1, max = 256))]
    pub(super) columns: Vec<SpreadsheetColumnCopy>,
    #[schemars(length(min = 1, max = 32))]
    pub(super) conditions: Vec<SpreadsheetFilterCondition>,
    #[serde(default)]
    pub(super) match_mode: SpreadsheetFilterMatchMode,
    #[serde(default)]
    pub(super) content_mode: SpreadsheetCopyContentMode,
}

#[derive(Debug, Clone)]
enum SpreadsheetMutation {
    WriteRows {
        sheet: String,
        start: CellAddress,
        rows: Vec<Vec<SpreadsheetCellInput>>,
    },
    WriteColumns {
        sheet: String,
        start: CellAddress,
        columns: Vec<Vec<SpreadsheetCellInput>>,
    },
    CopyRange {
        source_path: String,
        source_sheet: String,
        source_start: CellAddress,
        row_count: u32,
        column_count: u32,
        destination_sheet: String,
        destination_start: CellAddress,
        content_mode: SpreadsheetCopyContentMode,
    },
    CopyRows(SpreadsheetRowCopy),
    ConvertRange {
        sheet: String,
        range: CellRange,
        transforms: Vec<SpreadsheetValueTransform>,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum SpreadsheetToolInput {
    #[schemars(rename_all = "camelCase")]
    InspectDelimited {
        /// Real CSV/TSV path from the attachment manifest or filesystem.
        path: String,
        #[serde(default)]
        format: Option<DelimitedFormat>,
        #[serde(default)]
        header_row: u32,
        #[serde(default)]
        #[schemars(range(min = 1, max = 20))]
        sample_rows: Option<usize>,
        #[serde(default)]
        rstrip_tabs: bool,
    },
    #[schemars(rename_all = "camelCase")]
    Inspect {
        /// Real workbook path. Relative and absolute paths follow the active filesystem authority.
        path: String,
    },
    #[schemars(rename_all = "camelCase")]
    ListSheets { path: String },
    #[schemars(rename_all = "camelCase")]
    ReadRange {
        path: String,
        sheet: String,
        /// Inclusive zero-based range.
        range: CellRange,
    },
    #[schemars(rename_all = "camelCase")]
    ReadRanges {
        path: String,
        #[schemars(length(min = 1, max = 64))]
        ranges: Vec<SheetRangeRequest>,
    },
    #[schemars(rename_all = "camelCase")]
    ReadRows {
        path: String,
        sheet: String,
        start_row: u32,
        start_column: u32,
        #[schemars(range(min = 1))]
        row_count: u32,
        #[schemars(range(min = 1))]
        column_count: u32,
    },
    #[schemars(rename_all = "camelCase")]
    ReadColumns {
        path: String,
        sheet: String,
        start_row: u32,
        start_column: u32,
        #[schemars(range(min = 1))]
        row_count: u32,
        #[schemars(range(min = 1))]
        column_count: u32,
    },
    #[schemars(rename_all = "camelCase")]
    Find {
        path: String,
        #[serde(default)]
        sheet: Option<String>,
        #[serde(default)]
        range: Option<CellRange>,
        query: String,
        #[serde(default)]
        match_mode: Option<SpreadsheetTextMatchMode>,
        #[serde(default)]
        case_sensitive: bool,
        #[serde(default)]
        include_formulas: bool,
        #[serde(default)]
        #[schemars(range(min = 1, max = 1000))]
        max_results: Option<usize>,
    },
    #[schemars(rename_all = "camelCase")]
    FilterRows {
        path: String,
        sheet: String,
        range: CellRange,
        #[schemars(length(min = 1, max = 32))]
        conditions: Vec<SpreadsheetFilterCondition>,
        #[serde(default)]
        filter_match_mode: Option<SpreadsheetFilterMatchMode>,
        #[serde(default)]
        return_mode: SpreadsheetFilterReturnMode,
        #[serde(default)]
        #[schemars(range(min = 1, max = 2000))]
        max_results: Option<usize>,
    },
    #[schemars(rename_all = "camelCase")]
    Validate {
        /// Real workbook path from the attachment manifest or filesystem.
        path: String,
        #[serde(default)]
        expected_sheets: Vec<String>,
        #[serde(default)]
        expected_populated_cells: Option<u64>,
        #[serde(default)]
        sheets: Vec<SpreadsheetSheetValidation>,
    },
    #[schemars(rename_all = "camelCase")]
    ExportDelimited {
        /// Real source workbook path.
        path: String,
        /// Destination .csv or .tsv path.
        output_path: String,
        sheet: String,
        #[serde(default)]
        range: Option<CellRange>,
        #[serde(default)]
        format: Option<DelimitedFormat>,
        #[serde(default)]
        formula_mode: DelimitedFormulaMode,
    },
    #[schemars(rename_all = "camelCase")]
    Write {
        path: String,
        #[serde(default)]
        template: Option<String>,
        #[schemars(length(max = 256))]
        sheets: Vec<SheetWriteRequest>,
    },
    #[schemars(rename_all = "camelCase")]
    WriteRows {
        path: String,
        #[serde(default)]
        template: Option<String>,
        sheet: String,
        start: CellAddress,
        #[schemars(length(min = 1, max = 10000))]
        rows: Vec<Vec<SpreadsheetCellInput>>,
    },
    #[schemars(rename_all = "camelCase")]
    WriteColumns {
        path: String,
        #[serde(default)]
        template: Option<String>,
        sheet: String,
        start: CellAddress,
        #[schemars(length(min = 1, max = 256))]
        columns: Vec<Vec<SpreadsheetCellInput>>,
    },
    #[schemars(rename_all = "camelCase")]
    CopyRanges {
        path: String,
        #[serde(default)]
        template: Option<String>,
        #[schemars(length(min = 1, max = 64))]
        copies: Vec<SpreadsheetRangeCopy>,
    },
    #[schemars(rename_all = "camelCase")]
    CopyRows {
        path: String,
        #[serde(default)]
        template: Option<String>,
        copy: SpreadsheetRowCopy,
    },
    #[schemars(rename_all = "camelCase")]
    ConvertRanges {
        path: String,
        #[serde(default)]
        template: Option<String>,
        #[schemars(length(min = 1, max = 64))]
        conversions: Vec<SpreadsheetRangeConversion>,
    },
}

impl SpreadsheetToolInput {
    fn action(&self) -> SpreadsheetToolAction {
        match self {
            Self::InspectDelimited { .. } => SpreadsheetToolAction::InspectDelimited,
            Self::Inspect { .. } => SpreadsheetToolAction::Inspect,
            Self::ListSheets { .. } => SpreadsheetToolAction::ListSheets,
            Self::ReadRange { .. } => SpreadsheetToolAction::ReadRange,
            Self::ReadRanges { .. } => SpreadsheetToolAction::ReadRanges,
            Self::ReadRows { .. } => SpreadsheetToolAction::ReadRows,
            Self::ReadColumns { .. } => SpreadsheetToolAction::ReadColumns,
            Self::Find { .. } => SpreadsheetToolAction::Find,
            Self::FilterRows { .. } => SpreadsheetToolAction::FilterRows,
            Self::Validate { .. } => SpreadsheetToolAction::Validate,
            Self::ExportDelimited { .. } => SpreadsheetToolAction::ExportDelimited,
            Self::Write { .. } => SpreadsheetToolAction::Write,
            Self::WriteRows { .. } => SpreadsheetToolAction::WriteRows,
            Self::WriteColumns { .. } => SpreadsheetToolAction::WriteColumns,
            Self::CopyRanges { .. } => SpreadsheetToolAction::CopyRanges,
            Self::CopyRows { .. } => SpreadsheetToolAction::CopyRows,
            Self::ConvertRanges { .. } => SpreadsheetToolAction::ConvertRanges,
        }
    }

    fn into_execution_input(self) -> SpreadsheetExecutionInput {
        SpreadsheetExecutionInput::from(self)
    }
}

struct SpreadsheetExecutionInput {
    action: SpreadsheetToolAction,
    path: Option<String>,
    format: Option<DelimitedFormat>,
    header_row: u32,
    sample_rows: Option<usize>,
    rstrip_tabs: bool,
    sheet: Option<String>,
    range: Option<CellRange>,
    ranges: Vec<SheetRangeRequest>,
    start_row: Option<u32>,
    start_column: Option<u32>,
    row_count: Option<u32>,
    column_count: Option<u32>,
    query: Option<String>,
    match_mode: Option<SpreadsheetTextMatchMode>,
    case_sensitive: bool,
    include_formulas: bool,
    conditions: Vec<SpreadsheetFilterCondition>,
    filter_match_mode: Option<SpreadsheetFilterMatchMode>,
    filter_return_mode: SpreadsheetFilterReturnMode,
    max_results: Option<usize>,
    start: Option<CellAddress>,
    rows: Vec<Vec<SpreadsheetCellInput>>,
    columns: Vec<Vec<SpreadsheetCellInput>>,
    template: Option<String>,
    copies: Vec<SpreadsheetRangeCopy>,
    row_copies: Vec<SpreadsheetRowCopy>,
    conversions: Vec<SpreadsheetRangeConversion>,
    output_path: Option<String>,
    formula_mode: DelimitedFormulaMode,
    expected_sheets: Vec<String>,
    expected_populated_cells: Option<u64>,
    sheet_validations: Vec<SpreadsheetSheetValidation>,
    sheets: Vec<SheetWriteRequest>,
}

impl SpreadsheetExecutionInput {
    fn mutation_output_path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    fn direct_mutations(&self) -> anyhow::Result<Vec<SpreadsheetMutation>> {
        let operations = match self.action {
            SpreadsheetToolAction::WriteRows => vec![SpreadsheetMutation::WriteRows {
                sheet: required_typed_string(self.sheet.as_deref(), "sheet")?,
                start: self
                    .start
                    .context("spreadsheet write_rows requires start")?,
                rows: self.rows.clone(),
            }],
            SpreadsheetToolAction::WriteColumns => vec![SpreadsheetMutation::WriteColumns {
                sheet: required_typed_string(self.sheet.as_deref(), "sheet")?,
                start: self
                    .start
                    .context("spreadsheet write_columns requires start")?,
                columns: self.columns.clone(),
            }],
            SpreadsheetToolAction::CopyRanges => self
                .copies
                .iter()
                .cloned()
                .map(|copy| SpreadsheetMutation::CopyRange {
                    source_path: copy.source_path,
                    source_sheet: copy.source_sheet,
                    source_start: copy.source_start,
                    row_count: copy.row_count,
                    column_count: copy.column_count,
                    destination_sheet: copy.destination_sheet,
                    destination_start: copy.destination_start,
                    content_mode: copy.content_mode,
                })
                .collect(),
            SpreadsheetToolAction::CopyRows => self
                .row_copies
                .iter()
                .cloned()
                .map(SpreadsheetMutation::CopyRows)
                .collect(),
            SpreadsheetToolAction::ConvertRanges => self
                .conversions
                .iter()
                .cloned()
                .map(|conversion| SpreadsheetMutation::ConvertRange {
                    sheet: conversion.sheet,
                    range: conversion.range,
                    transforms: conversion.transforms,
                })
                .collect(),
            _ => anyhow::bail!("spreadsheet action is not an atomic mutation"),
        };
        anyhow::ensure!(
            !operations.is_empty(),
            "spreadsheet mutation list must not be empty"
        );
        Ok(operations)
    }
}

impl From<SpreadsheetToolInput> for SpreadsheetExecutionInput {
    fn from(input: SpreadsheetToolInput) -> Self {
        let mut execution = Self {
            action: input.action(),
            path: None,
            format: None,
            header_row: 0,
            sample_rows: None,
            rstrip_tabs: false,
            sheet: None,
            range: None,
            ranges: Vec::new(),
            start_row: None,
            start_column: None,
            row_count: None,
            column_count: None,
            query: None,
            match_mode: None,
            case_sensitive: false,
            include_formulas: false,
            conditions: Vec::new(),
            filter_match_mode: None,
            filter_return_mode: SpreadsheetFilterReturnMode::Summary,
            max_results: None,
            start: None,
            rows: Vec::new(),
            columns: Vec::new(),
            template: None,
            copies: Vec::new(),
            row_copies: Vec::new(),
            conversions: Vec::new(),
            output_path: None,
            formula_mode: DelimitedFormulaMode::Values,
            expected_sheets: Vec::new(),
            expected_populated_cells: None,
            sheet_validations: Vec::new(),
            sheets: Vec::new(),
        };
        match input {
            SpreadsheetToolInput::InspectDelimited {
                path,
                format,
                header_row,
                sample_rows,
                rstrip_tabs,
            } => {
                execution.path = Some(path);
                execution.format = format;
                execution.header_row = header_row;
                execution.sample_rows = sample_rows;
                execution.rstrip_tabs = rstrip_tabs;
            }
            SpreadsheetToolInput::Inspect { path } | SpreadsheetToolInput::ListSheets { path } => {
                execution.path = Some(path);
            }
            SpreadsheetToolInput::ReadRange { path, sheet, range } => {
                execution.path = Some(path);
                execution.sheet = Some(sheet);
                execution.range = Some(range);
            }
            SpreadsheetToolInput::ReadRanges { path, ranges } => {
                execution.path = Some(path);
                execution.ranges = ranges;
            }
            SpreadsheetToolInput::ReadRows {
                path,
                sheet,
                start_row,
                start_column,
                row_count,
                column_count,
            }
            | SpreadsheetToolInput::ReadColumns {
                path,
                sheet,
                start_row,
                start_column,
                row_count,
                column_count,
            } => {
                execution.path = Some(path);
                execution.sheet = Some(sheet);
                execution.start_row = Some(start_row);
                execution.start_column = Some(start_column);
                execution.row_count = Some(row_count);
                execution.column_count = Some(column_count);
            }
            SpreadsheetToolInput::Find {
                path,
                sheet,
                range,
                query,
                match_mode,
                case_sensitive,
                include_formulas,
                max_results,
            } => {
                execution.path = Some(path);
                execution.sheet = sheet;
                execution.range = range;
                execution.query = Some(query);
                execution.match_mode = match_mode;
                execution.case_sensitive = case_sensitive;
                execution.include_formulas = include_formulas;
                execution.max_results = max_results;
            }
            SpreadsheetToolInput::FilterRows {
                path,
                sheet,
                range,
                conditions,
                filter_match_mode,
                return_mode,
                max_results,
            } => {
                execution.path = Some(path);
                execution.sheet = Some(sheet);
                execution.range = Some(range);
                execution.conditions = conditions;
                execution.filter_match_mode = filter_match_mode;
                execution.filter_return_mode = return_mode;
                execution.max_results = max_results;
            }
            SpreadsheetToolInput::Validate {
                path,
                expected_sheets,
                expected_populated_cells,
                sheets,
            } => {
                execution.path = Some(path);
                execution.expected_sheets = expected_sheets;
                execution.expected_populated_cells = expected_populated_cells;
                execution.sheet_validations = sheets;
            }
            SpreadsheetToolInput::ExportDelimited {
                path,
                output_path,
                sheet,
                range,
                format,
                formula_mode,
            } => {
                execution.path = Some(path);
                execution.output_path = Some(output_path);
                execution.sheet = Some(sheet);
                execution.range = range;
                execution.format = format;
                execution.formula_mode = formula_mode;
            }
            SpreadsheetToolInput::Write {
                path,
                template,
                sheets,
            } => {
                execution.path = Some(path);
                execution.template = template;
                execution.sheets = sheets;
            }
            SpreadsheetToolInput::WriteRows {
                path,
                template,
                sheet,
                start,
                rows,
            } => {
                execution.path = Some(path);
                execution.template = template;
                execution.sheet = Some(sheet);
                execution.start = Some(start);
                execution.rows = rows;
            }
            SpreadsheetToolInput::WriteColumns {
                path,
                template,
                sheet,
                start,
                columns,
            } => {
                execution.path = Some(path);
                execution.template = template;
                execution.sheet = Some(sheet);
                execution.start = Some(start);
                execution.columns = columns;
            }
            SpreadsheetToolInput::CopyRanges {
                path,
                template,
                copies,
            } => {
                execution.path = Some(path);
                execution.template = template;
                execution.copies = copies;
            }
            SpreadsheetToolInput::CopyRows {
                path,
                template,
                copy,
            } => {
                execution.path = Some(path);
                execution.template = template;
                execution.row_copies.push(copy);
            }
            SpreadsheetToolInput::ConvertRanges {
                path,
                template,
                conversions,
            } => {
                execution.path = Some(path);
                execution.template = template;
                execution.conversions = conversions;
            }
        }
        execution
    }
}

pub(super) async fn execute_spreadsheet_backend(
    call_id: Uuid,
    input: Value,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let input: SpreadsheetToolInput = decode_typed_tool_input("spreadsheet backend", input)?;
    let action = input.action();
    let input = input.into_execution_input();
    match action {
        SpreadsheetToolAction::InspectDelimited
        | SpreadsheetToolAction::Inspect
        | SpreadsheetToolAction::ListSheets
        | SpreadsheetToolAction::ReadRange
        | SpreadsheetToolAction::ReadRanges
        | SpreadsheetToolAction::ReadRows
        | SpreadsheetToolAction::ReadColumns
        | SpreadsheetToolAction::Find
        | SpreadsheetToolAction::FilterRows
        | SpreadsheetToolAction::Validate => execute_spreadsheet_read(call_id, input, ctx).await,
        SpreadsheetToolAction::ExportDelimited => {
            execute_spreadsheet_export_delimited(call_id, input, ctx).await
        }
        SpreadsheetToolAction::Write => execute_spreadsheet_write(call_id, input, ctx).await,
        SpreadsheetToolAction::WriteRows
        | SpreadsheetToolAction::WriteColumns
        | SpreadsheetToolAction::CopyRanges
        | SpreadsheetToolAction::CopyRows
        | SpreadsheetToolAction::ConvertRanges => {
            execute_spreadsheet_mutations(call_id, input, ctx).await
        }
    }
}

fn spreadsheet_operation_source_path(operation: &SpreadsheetMutation) -> Option<&str> {
    match operation {
        SpreadsheetMutation::CopyRange { source_path, .. } => Some(source_path),
        SpreadsheetMutation::CopyRows(copy) => Some(&copy.source_path),
        SpreadsheetMutation::WriteRows { .. }
        | SpreadsheetMutation::WriteColumns { .. }
        | SpreadsheetMutation::ConvertRange { .. } => None,
    }
}

async fn execute_spreadsheet_read(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let mut action = input.action;
    let mut reads_delimited = action == SpreadsheetToolAction::InspectDelimited;
    let source_path = required_typed_string(input.path.as_deref(), "path")?;
    let logical_path = normalize_workspace_path(&ctx.workspace_root, &source_path)?;
    enforce_read_policy(&ctx, &logical_path)?;
    let resolved_path = ctx.environment.resolve_read_path(&logical_path)?;
    let source = ctx
        .environment
        .read_file(FileReadRequest::new(&resolved_path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
        .await?;
    let resolved_path = source.path;
    let source_bytes = source.bytes;
    if action == SpreadsheetToolAction::Inspect
        && SpreadsheetFileFormat::from_path(&resolved_path)
            .is_some_and(SpreadsheetFileFormat::is_delimited)
    {
        action = SpreadsheetToolAction::InspectDelimited;
        reads_delimited = true;
    }
    let resolved_delimited_format = if reads_delimited {
        Some(resolve_delimited_format(&resolved_path, input.format)?)
    } else {
        ensure_workbook_path(&resolved_path)?;
        None
    };
    let source_path = resolved_path.clone();
    let format = resolved_delimited_format.or(input.format);
    let header_row = input.header_row;
    let sample_rows = input.sample_rows;
    let rstrip_tabs = input.rstrip_tabs;
    let sheet = input.sheet;
    let range = input.range;
    let ranges = input.ranges;
    let start_row = input.start_row;
    let start_column = input.start_column;
    let row_count = input.row_count;
    let column_count = input.column_count;
    let query = input.query;
    let match_mode = input.match_mode;
    let case_sensitive = input.case_sensitive;
    let include_formulas = input.include_formulas;
    let conditions = input.conditions;
    let filter_match_mode = input.filter_match_mode;
    let filter_return_mode = input.filter_return_mode;
    let max_results = input.max_results;
    let expected_sheets = input.expected_sheets;
    let expected_populated_cells = input.expected_populated_cells;
    let sheet_validations = input.sheet_validations;
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_input_name = if action == SpreadsheetToolAction::InspectDelimited {
            match format.unwrap_or_default() {
                DelimitedFormat::Csv => "input.csv".to_string(),
                DelimitedFormat::Tsv => "input.tsv".to_string(),
            }
        } else {
            staging_file_name("input", &source_path, "xlsx")
        };
        let staged_input = staging.path(&staged_input_name);
        fs::write(&staged_input, source_bytes)
            .with_context(|| format!("failed to stage {}", source_path.display()))?;
        let action = match action {
            SpreadsheetToolAction::InspectDelimited => {
                SpreadsheetAction::InspectDelimited(InspectDelimitedRequest {
                    path: staged_input,
                    format,
                    header_row,
                    sample_rows: sample_rows.unwrap_or(5),
                    rstrip_tabs,
                })
            }
            SpreadsheetToolAction::Inspect => {
                SpreadsheetAction::InspectWorkbook(InspectWorkbookRequest { path: staged_input })
            }
            SpreadsheetToolAction::ListSheets => {
                SpreadsheetAction::ListSheets(ListSheetsRequest { path: staged_input })
            }
            SpreadsheetToolAction::ReadRange => SpreadsheetAction::ReadRange(ReadRangeRequest {
                path: staged_input,
                sheet: sheet
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .context("spreadsheet read_range requires sheet")?,
                range: range.context("spreadsheet read_range requires range")?,
            }),
            SpreadsheetToolAction::ReadRanges => {
                anyhow::ensure!(
                    !ranges.is_empty(),
                    "spreadsheet read_ranges requires at least one range"
                );
                SpreadsheetAction::ReadRanges(ReadRangesRequest {
                    path: staged_input,
                    ranges,
                })
            }
            SpreadsheetToolAction::ReadRows | SpreadsheetToolAction::ReadColumns => {
                let range = counted_spreadsheet_range(
                    start_row.context("spreadsheet row/column read requires startRow")?,
                    start_column.context("spreadsheet row/column read requires startColumn")?,
                    row_count.context("spreadsheet row/column read requires rowCount")?,
                    column_count.context("spreadsheet row/column read requires columnCount")?,
                )?;
                SpreadsheetAction::ReadRange(ReadRangeRequest {
                    path: staged_input,
                    sheet: sheet
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .context("spreadsheet row/column read requires sheet")?,
                    range,
                })
            }
            SpreadsheetToolAction::Find => SpreadsheetAction::FindCells(FindCellsRequest {
                path: staged_input,
                sheet: sheet
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty()),
                range,
                query: query
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .context("spreadsheet find requires query")?,
                match_mode: match_mode.unwrap_or_default(),
                case_sensitive,
                include_formulas,
                max_results: max_results.unwrap_or(100),
            }),
            SpreadsheetToolAction::FilterRows => {
                anyhow::ensure!(
                    !conditions.is_empty(),
                    "spreadsheet filter_rows requires conditions"
                );
                SpreadsheetAction::FilterRows(FilterRowsRequest {
                    path: staged_input,
                    sheet: sheet
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                        .context("spreadsheet filter_rows requires sheet")?,
                    range: range.context("spreadsheet filter_rows requires range")?,
                    conditions,
                    match_mode: filter_match_mode.unwrap_or_default(),
                    return_mode: filter_return_mode,
                    max_results: max_results.unwrap_or(100),
                })
            }
            SpreadsheetToolAction::Validate => {
                SpreadsheetAction::ValidateWorkbook(ValidateWorkbookRequest {
                    path: staged_input,
                    expected_sheets,
                    expected_populated_cells,
                    sheets: sheet_validations,
                })
            }
            SpreadsheetToolAction::ExportDelimited
            | SpreadsheetToolAction::Write
            | SpreadsheetToolAction::WriteRows
            | SpreadsheetToolAction::WriteColumns
            | SpreadsheetToolAction::CopyRanges
            | SpreadsheetToolAction::CopyRows
            | SpreadsheetToolAction::ConvertRanges => unreachable!(),
        };
        Ok(execute_spreadsheet(SpreadsheetRequest { action }))
    })
    .await
    .context("spreadsheet worker task failed")??;
    let mut result = match outcome {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    remap_spreadsheet_paths(&mut result, Some(&resolved_path), None);
    spreadsheet_success_result(call_id, result, None)
}

async fn execute_spreadsheet_export_delimited(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let output_relative = required_typed_string(input.output_path.as_deref(), "outputPath")?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, &output_relative)?;
    let format = resolve_delimited_format(&output_path, input.format)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), &ctx)?;
    let original_output = read_optional(ctx.environment.as_ref(), &output_path).await?;

    let source_path = required_typed_string(input.path.as_deref(), "path")?;
    let logical_source_path = normalize_workspace_path(&ctx.workspace_root, &source_path)?;
    enforce_read_policy(&ctx, &logical_source_path)?;
    let resolved_source_path = ctx.environment.resolve_read_path(&logical_source_path)?;
    ensure_workbook_path(&resolved_source_path)?;
    let source = ctx
        .environment
        .read_file(
            FileReadRequest::new(&resolved_source_path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES),
        )
        .await?;
    let format = Some(format);
    let sheet = required_typed_string(input.sheet.as_deref(), "sheet")?;
    let range = input.range;
    let formula_mode = input.formula_mode;
    let staged = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_source_name = staging_file_name("source", &source.path, "xlsx");
        let staged_source = staging.path(&staged_source_name);
        let staged_output = staging.path(match format.unwrap_or_default() {
            DelimitedFormat::Csv => "output.csv",
            DelimitedFormat::Tsv => "output.tsv",
        });
        fs::write(&staged_source, source.bytes)
            .with_context(|| format!("failed to stage {}", source.path.display()))?;
        let outcome = execute_spreadsheet(SpreadsheetRequest {
            action: SpreadsheetAction::ExportDelimited(ExportDelimitedRequest {
                path: staged_source,
                output: staged_output.clone(),
                sheet,
                range,
                format,
                formula_mode,
            }),
        });
        match outcome {
            Ok(result) => {
                let bytes = fs::read(&staged_output)
                    .with_context(|| format!("failed to read {}", staged_output.display()))?;
                Ok(Ok((result, bytes, source.path)))
            }
            Err(error) => Ok(Err(error)),
        }
    })
    .await
    .context("spreadsheet export_delimited worker task failed")??;
    let (mut result, bytes, resolved_source_path) = match staged {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
        &output_path,
        original_output,
        bytes,
    )])?;
    ctx.commit_file_mutations(&batch).await?;
    remap_spreadsheet_paths(&mut result, Some(&resolved_source_path), Some(&output_path));
    spreadsheet_success_result(call_id, result, Some(output_path))
}

fn counted_spreadsheet_range(
    start_row: u32,
    start_column: u32,
    row_count: u32,
    column_count: u32,
) -> anyhow::Result<CellRange> {
    anyhow::ensure!(row_count > 0, "spreadsheet rowCount must be at least 1");
    anyhow::ensure!(
        column_count > 0,
        "spreadsheet columnCount must be at least 1"
    );
    let end_row = start_row
        .checked_add(row_count - 1)
        .context("spreadsheet row range overflow")?;
    let end_column = start_column
        .checked_add(column_count - 1)
        .context("spreadsheet column range overflow")?;
    Ok(CellRange {
        start: CellAddress {
            row: start_row,
            column: start_column,
        },
        end: CellAddress {
            row: end_row,
            column: end_column,
        },
    })
}

async fn execute_spreadsheet_write(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let output_relative = input
        .mutation_output_path()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("spreadsheet write requires path")?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, output_relative)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), &ctx)?;

    let (original, source) =
        read_mutation_base(&ctx, &output_path, input.template.as_deref()).await?;

    let sheets = input.sheets;
    let staged_output_format_path = output_path.clone();
    let staged = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_source = if let Some(source) = source {
            let source_name = staging_file_name("source", &source.path, "xlsx");
            let path = staging.path(&source_name);
            fs::write(&path, source.bytes)
                .with_context(|| format!("failed to stage {}", source.path.display()))?;
            Some(path)
        } else {
            None
        };
        let output_name = staging_file_name("output", &staged_output_format_path, "xlsx");
        let staged_output = staging.path(&output_name);
        let outcome = execute_spreadsheet(SpreadsheetRequest {
            action: SpreadsheetAction::WriteWorkbook(WriteWorkbookRequest {
                source: staged_source,
                output: staged_output.clone(),
                sheets,
            }),
        });
        match outcome {
            Ok(result) => {
                let bytes = fs::read(&staged_output)
                    .with_context(|| format!("failed to read {}", staged_output.display()))?;
                Ok(Ok((result, bytes)))
            }
            Err(error) => Ok(Err(error)),
        }
    })
    .await
    .context("spreadsheet worker task failed")??;
    let (mut result, bytes) = match staged {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
        &output_path,
        original,
        bytes,
    )])?;
    ctx.commit_file_mutations(&batch).await?;
    remap_spreadsheet_paths(&mut result, None, Some(&output_path));
    spreadsheet_success_result(call_id, result, Some(output_path))
}

async fn execute_spreadsheet_mutations(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let output_relative = input
        .mutation_output_path()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("spreadsheet mutation requires path")?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, output_relative)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), &ctx)?;

    let operations = input.direct_mutations()?;
    let (original, base_source) =
        read_mutation_base(&ctx, &output_path, input.template.as_deref()).await?;

    let mut copy_sources = BTreeMap::<String, (PathBuf, Vec<u8>)>::new();
    for relative in operations
        .iter()
        .filter_map(spreadsheet_operation_source_path)
    {
        let key = relative.trim().to_string();
        anyhow::ensure!(
            !key.is_empty(),
            "spreadsheet copy sourcePath must not be empty"
        );
        if copy_sources.contains_key(&key) {
            continue;
        }
        let logical_path = normalize_workspace_path(&ctx.workspace_root, &key)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;
        ensure_workbook_path(&path)?;
        let read = ctx
            .environment
            .read_file(FileReadRequest::new(&path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
            .await?;
        copy_sources.insert(key, (read.path, read.bytes));
    }

    let staged_output_format_path = output_path.clone();
    let staged = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_base = if let Some(source) = base_source {
            let source_name = staging_file_name("base", &source.path, "xlsx");
            let path = staging.path(&source_name);
            fs::write(&path, source.bytes)
                .with_context(|| format!("failed to stage {}", source.path.display()))?;
            Some(path)
        } else {
            None
        };
        let mut staged_copy_sources = BTreeMap::new();
        for (index, (logical, (original, bytes))) in copy_sources.into_iter().enumerate() {
            let source_name = staging_file_name(&format!("copy-source-{index}"), &original, "xlsx");
            let path = staging.path(&source_name);
            fs::write(&path, bytes)
                .with_context(|| format!("failed to stage {}", original.display()))?;
            staged_copy_sources.insert(logical, path);
        }
        let materialized = materialize_spreadsheet_operations(
            &operations,
            staged_base.as_deref(),
            &staged_copy_sources,
        )?;
        let output_name = staging_file_name("output", &staged_output_format_path, "xlsx");
        let staged_output = staging.path(&output_name);
        let outcome = execute_spreadsheet(SpreadsheetRequest {
            action: SpreadsheetAction::WriteWorkbook(WriteWorkbookRequest {
                source: staged_base,
                output: staged_output.clone(),
                sheets: materialized.sheets,
            }),
        });
        match outcome {
            Ok(result) => {
                let final_output = if materialized.formats.is_empty() {
                    staged_output
                } else {
                    let formatted_output = staging.path(&staging_file_name(
                        "formatted-output",
                        &staged_output_format_path,
                        "xlsx",
                    ));
                    if let Err(error) = edit_workbook_structure(&EditWorkbookStructureRequest {
                        source: staged_output,
                        output: formatted_output.clone(),
                        operations: materialized.formats,
                    }) {
                        return Ok(Err(error));
                    }
                    formatted_output
                };
                let bytes = fs::read(&final_output)
                    .with_context(|| format!("failed to read {}", final_output.display()))?;
                Ok(Ok((result, bytes)))
            }
            Err(error) => Ok(Err(error)),
        }
    })
    .await
    .context("spreadsheet mutation worker task failed")??;
    let (mut result, bytes) = match staged {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
        &output_path,
        original,
        bytes,
    )])?;
    ctx.commit_file_mutations(&batch).await?;
    remap_spreadsheet_paths(&mut result, None, Some(&output_path));
    spreadsheet_success_result(call_id, result, Some(output_path))
}

async fn read_mutation_base(
    ctx: &ToolInvocationContext,
    output_path: &Path,
    template_requested: Option<&str>,
) -> anyhow::Result<(Option<Vec<u8>>, Option<crate::execution::FileReadResult>)> {
    enforce_read_policy(ctx, output_path)?;
    let original = read_optional(ctx.environment.as_ref(), output_path).await?;
    let source = if let Some(template_requested) = template_requested {
        let template_path = normalize_workspace_path(&ctx.workspace_root, template_requested)?;
        enforce_read_policy(ctx, &template_path)?;
        let resolved_template = ctx.environment.resolve_read_path(&template_path)?;
        ensure_workbook_path(&resolved_template)?;
        Some(
            ctx.environment
                .read_file(
                    FileReadRequest::new(&resolved_template)
                        .with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES),
                )
                .await?,
        )
    } else {
        original
            .clone()
            .map(|bytes| crate::execution::FileReadResult {
                path: output_path.to_path_buf(),
                bytes,
            })
    };
    Ok((original, source))
}

struct MaterializedSpreadsheetOperations {
    sheets: Vec<SheetWriteRequest>,
    formats: Vec<SpreadsheetStructureOperation>,
}

fn materialize_spreadsheet_operations(
    operations: &[SpreadsheetMutation],
    base_source: Option<&Path>,
    copy_sources: &BTreeMap<String, PathBuf>,
) -> anyhow::Result<MaterializedSpreadsheetOperations> {
    let mut copy_batches = BTreeMap::<String, Vec<(usize, SheetRangeRequest)>>::new();
    let mut source_header_batches = BTreeMap::<String, Vec<(usize, SheetRangeRequest)>>::new();
    let mut conversion_batch = Vec::<(usize, SheetRangeRequest)>::new();
    let mut destination_header_batch = Vec::<(usize, SheetRangeRequest)>::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            SpreadsheetMutation::CopyRange {
                source_path,
                source_sheet,
                source_start,
                row_count,
                column_count,
                ..
            } => {
                let range = counted_spreadsheet_range(
                    source_start.row,
                    source_start.column,
                    *row_count,
                    *column_count,
                )?;
                copy_batches
                    .entry(source_path.trim().to_string())
                    .or_default()
                    .push((
                        index,
                        SheetRangeRequest {
                            sheet: source_sheet.clone(),
                            range,
                        },
                    ));
            }
            SpreadsheetMutation::ConvertRange { sheet, range, .. } => {
                conversion_batch.push((
                    index,
                    SheetRangeRequest {
                        sheet: sheet.clone(),
                        range: *range,
                    },
                ));
            }
            SpreadsheetMutation::CopyRows(copy) => {
                let source_start_column = copy
                    .columns
                    .iter()
                    .map(|column| column.source_column)
                    .chain(copy.conditions.iter().map(|condition| condition.column))
                    .min()
                    .context("spreadsheet row-copy requires source columns")?;
                let source_end_column = copy
                    .columns
                    .iter()
                    .map(|column| column.source_column)
                    .chain(copy.conditions.iter().map(|condition| condition.column))
                    .max()
                    .context("spreadsheet row-copy requires source columns")?;
                source_header_batches
                    .entry(copy.source_path.trim().to_string())
                    .or_default()
                    .push((
                        index,
                        SheetRangeRequest {
                            sheet: copy.source_sheet.clone(),
                            range: CellRange {
                                start: CellAddress {
                                    row: copy.source_header_row,
                                    column: source_start_column,
                                },
                                end: CellAddress {
                                    row: copy.source_header_row,
                                    column: source_end_column,
                                },
                            },
                        },
                    ));
                let start_column = copy
                    .columns
                    .iter()
                    .map(|column| column.destination_column)
                    .min()
                    .context("spreadsheet row-copy requires destination columns")?;
                let end_column = copy
                    .columns
                    .iter()
                    .map(|column| column.destination_column)
                    .max()
                    .context("spreadsheet row-copy requires destination columns")?;
                destination_header_batch.push((
                    index,
                    SheetRangeRequest {
                        sheet: copy.destination_sheet.clone(),
                        range: CellRange {
                            start: CellAddress {
                                row: copy.destination_header_row,
                                column: start_column,
                            },
                            end: CellAddress {
                                row: copy.destination_header_row,
                                column: end_column,
                            },
                        },
                    },
                ));
            }
            SpreadsheetMutation::WriteRows { .. } | SpreadsheetMutation::WriteColumns { .. } => {}
        }
    }

    let mut loaded_ranges = BTreeMap::new();
    for (source_path, batch) in copy_batches {
        let staged_source = copy_sources
            .get(&source_path)
            .with_context(|| format!("spreadsheet copy source {source_path:?} was not staged"))?;
        let reads = crate::spreadsheet::read_ranges_for_mutation(&ReadRangesRequest {
            path: staged_source.clone(),
            ranges: batch.iter().map(|(_, range)| range.clone()).collect(),
        })?;
        for ((index, _), read) in batch.into_iter().zip(reads.ranges) {
            loaded_ranges.insert(index, read);
        }
    }
    let mut source_headers = BTreeMap::new();
    for (source_path, batch) in source_header_batches {
        let staged_source = copy_sources.get(&source_path).with_context(|| {
            format!("spreadsheet row-copy source {source_path:?} was not staged")
        })?;
        let reads = crate::spreadsheet::read_ranges_for_mutation(&ReadRangesRequest {
            path: staged_source.clone(),
            ranges: batch.iter().map(|(_, range)| range.clone()).collect(),
        })?;
        for ((index, _), read) in batch.into_iter().zip(reads.ranges) {
            source_headers.insert(index, read);
        }
    }
    if !conversion_batch.is_empty() {
        let source = base_source
            .context("spreadsheet convert_ranges requires an existing destination workbook")?;
        let reads = crate::spreadsheet::read_ranges_for_mutation(&ReadRangesRequest {
            path: source.to_path_buf(),
            ranges: conversion_batch
                .iter()
                .map(|(_, range)| range.clone())
                .collect(),
        })?;
        for ((index, _), read) in conversion_batch.into_iter().zip(reads.ranges) {
            loaded_ranges.insert(index, read);
        }
    }
    let mut destination_headers = BTreeMap::new();
    if !destination_header_batch.is_empty() {
        let source = base_source
            .context("spreadsheet copy_rows requires an existing destination workbook")?;
        let reads = crate::spreadsheet::read_ranges_for_mutation(&ReadRangesRequest {
            path: source.to_path_buf(),
            ranges: destination_header_batch
                .iter()
                .map(|(_, range)| range.clone())
                .collect(),
        })?;
        for ((index, _), read) in destination_header_batch.into_iter().zip(reads.ranges) {
            destination_headers.insert(index, read);
        }
    }

    let mut updates = BTreeMap::<String, Vec<CellUpdate>>::new();
    let mut formats = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            SpreadsheetMutation::WriteRows { sheet, start, rows } => {
                append_row_updates(&mut updates, sheet, *start, rows)?;
            }
            SpreadsheetMutation::WriteColumns {
                sheet,
                start,
                columns,
            } => {
                append_column_updates(&mut updates, sheet, *start, columns)?;
            }
            SpreadsheetMutation::CopyRange {
                destination_sheet,
                destination_start,
                content_mode,
                ..
            } => {
                let read = loaded_ranges
                    .remove(&index)
                    .context("spreadsheet copy range was not loaded")?;
                let rows = read
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| spreadsheet_cell_to_input(cell, *content_mode))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                append_row_updates(&mut updates, destination_sheet, *destination_start, &rows)?;
            }
            SpreadsheetMutation::CopyRows(copy) => {
                let source_start_row = copy.source_data_row;
                anyhow::ensure!(
                    source_start_row > copy.source_header_row && source_start_row < EXCEL_MAX_ROWS,
                    "source_data_row must be after source_header_row"
                );
                let source_header = source_headers
                    .remove(&index)
                    .context("spreadsheet row-copy source header was not loaded")?;
                for source_column in copy
                    .columns
                    .iter()
                    .map(|column| column.source_column)
                    .chain(copy.conditions.iter().map(|condition| condition.column))
                {
                    let offset = usize::try_from(source_column - source_header.range.start.column)?;
                    let cell = source_header
                        .rows
                        .first()
                        .and_then(|row| row.get(offset))
                        .context("spreadsheet row-copy source header cell is missing")?;
                    anyhow::ensure!(
                        !spreadsheet_cell_is_blank(cell),
                        "source_header_row does not contain a header in referenced column {}",
                        format_a1_address(CellAddress {
                            row: copy.source_header_row,
                            column: source_column,
                        })
                    );
                }
                let source_range = CellRange {
                    start: CellAddress {
                        row: source_start_row,
                        column: source_header.range.start.column,
                    },
                    end: CellAddress {
                        row: EXCEL_MAX_ROWS - 1,
                        column: source_header.range.end.column,
                    },
                };
                let destination_start_row = copy
                    .destination_header_row
                    .checked_add(1)
                    .context("spreadsheet row-copy destination row overflow")?;
                let header = destination_headers
                    .remove(&index)
                    .context("spreadsheet row-copy destination header was not loaded")?;
                for column in &copy.columns {
                    let offset =
                        usize::try_from(column.destination_column - header.range.start.column)?;
                    let cell = header
                        .rows
                        .first()
                        .and_then(|row| row.get(offset))
                        .context("spreadsheet row-copy destination header cell is missing")?;
                    anyhow::ensure!(
                        !spreadsheet_cell_is_blank(cell),
                        "destination_header_row does not contain a header in mapped column {}",
                        format_a1_address(CellAddress {
                            row: copy.destination_header_row,
                            column: column.destination_column,
                        })
                    );
                }
                let staged_source =
                    copy_sources.get(copy.source_path.trim()).with_context(|| {
                        format!(
                            "spreadsheet row-copy source {:?} was not staged",
                            copy.source_path
                        )
                    })?;
                let filtered = crate::spreadsheet::filter_rows(&FilterRowsRequest {
                    path: staged_source.clone(),
                    sheet: copy.source_sheet.clone(),
                    range: source_range,
                    conditions: copy.conditions.clone(),
                    match_mode: copy.match_mode,
                    return_mode: SpreadsheetFilterReturnMode::Rows,
                    max_results: crate::spreadsheet::MAX_FILTER_RESULTS,
                })?;
                anyhow::ensure!(
                    !filtered.truncated,
                    "more than {} source rows matched; split the source range",
                    crate::spreadsheet::MAX_FILTER_RESULTS
                );
                anyhow::ensure!(
                    filtered.rows.len() == filtered.matched_row_indices.len(),
                    "spreadsheet row-copy result is internally inconsistent"
                );
                let sheet_updates = updates.entry(copy.destination_sheet.clone()).or_default();
                for (row_offset, row) in filtered.rows.iter().enumerate() {
                    let destination_row = destination_start_row
                        .checked_add(u32::try_from(row_offset)?)
                        .context("spreadsheet row-copy destination row overflow")?;
                    for column in &copy.columns {
                        anyhow::ensure!(
                            column.source_column >= source_range.start.column
                                && column.source_column <= source_range.end.column,
                            "spreadsheet row-copy source column is outside source_range"
                        );
                        let source_offset =
                            usize::try_from(column.source_column - source_range.start.column)?;
                        let cell = row
                            .get(source_offset)
                            .context("spreadsheet row-copy source cell is missing")?;
                        let input = spreadsheet_cell_to_input(cell, copy.content_mode);
                        let value = if matches!(cell.value, SpreadsheetCellValue::Empty)
                            || cell.formula.is_some()
                            || column.transforms.is_empty()
                        {
                            input
                        } else {
                            transform_cell_input(
                                input,
                                &column.transforms,
                                filtered.matched_row_indices[row_offset],
                                column.source_column,
                            )?
                        };
                        sheet_updates.push(CellUpdate {
                            address: CellAddress {
                                row: destination_row,
                                column: column.destination_column,
                            },
                            value,
                            style_from: Some(CellAddress {
                                row: destination_start_row,
                                column: column.destination_column,
                            }),
                        });
                    }
                }
                if !filtered.rows.is_empty() {
                    let end_row = copy
                        .destination_header_row
                        .checked_add(1)
                        .context("spreadsheet row-copy destination row overflow")?
                        .checked_add(u32::try_from(filtered.rows.len() - 1)?)
                        .context("spreadsheet row-copy destination row overflow")?;
                    for column in &copy.columns {
                        if let Some(number_format) = transform_number_format(&column.transforms)? {
                            formats.push(SpreadsheetStructureOperation::SetNumberFormat {
                                sheet: copy.destination_sheet.clone(),
                                range: CellRange {
                                    start: CellAddress {
                                        row: destination_start_row,
                                        column: column.destination_column,
                                    },
                                    end: CellAddress {
                                        row: end_row,
                                        column: column.destination_column,
                                    },
                                },
                                number_format,
                            });
                        }
                    }
                }
            }
            SpreadsheetMutation::ConvertRange {
                sheet,
                range,
                transforms,
            } => {
                let read = loaded_ranges
                    .remove(&index)
                    .context("spreadsheet conversion range was not loaded")?;
                let rows = read
                    .rows
                    .into_iter()
                    .enumerate()
                    .map(|(row_offset, row)| {
                        row.into_iter()
                            .enumerate()
                            .map(|(column_offset, cell)| {
                                let input = spreadsheet_cell_to_input(
                                    &cell,
                                    SpreadsheetCopyContentMode::ValuesAndFormulas,
                                );
                                if matches!(cell.value, SpreadsheetCellValue::Empty)
                                    || cell.formula.is_some()
                                {
                                    return Ok(input);
                                }
                                transform_cell_input(
                                    input,
                                    transforms,
                                    range.start.row + row_offset as u32,
                                    range.start.column + column_offset as u32,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                append_row_updates(&mut updates, sheet, range.start, &rows)?;
                if let Some(number_format) = transform_number_format(transforms)? {
                    formats.push(SpreadsheetStructureOperation::SetNumberFormat {
                        sheet: sheet.clone(),
                        range: *range,
                        number_format,
                    });
                }
            }
        }
    }
    Ok(MaterializedSpreadsheetOperations {
        sheets: updates
            .into_iter()
            .map(|(name, cells)| SheetWriteRequest {
                name,
                visibility: None,
                cells,
            })
            .collect(),
        formats,
    })
}

fn append_row_updates(
    updates: &mut BTreeMap<String, Vec<CellUpdate>>,
    sheet: &str,
    start: CellAddress,
    rows: &[Vec<SpreadsheetCellInput>],
) -> anyhow::Result<()> {
    let target = updates.entry(sheet.to_string()).or_default();
    for (row_offset, row) in rows.iter().enumerate() {
        let row_offset =
            u32::try_from(row_offset).context("spreadsheet row offset is too large")?;
        let address_row = start
            .row
            .checked_add(row_offset)
            .context("spreadsheet destination row overflow")?;
        for (column_offset, value) in row.iter().enumerate() {
            let column_offset =
                u32::try_from(column_offset).context("spreadsheet column offset is too large")?;
            target.push(CellUpdate {
                address: CellAddress {
                    row: address_row,
                    column: start
                        .column
                        .checked_add(column_offset)
                        .context("spreadsheet destination column overflow")?,
                },
                value: value.clone(),
                style_from: None,
            });
        }
    }
    Ok(())
}

fn append_column_updates(
    updates: &mut BTreeMap<String, Vec<CellUpdate>>,
    sheet: &str,
    start: CellAddress,
    columns: &[Vec<SpreadsheetCellInput>],
) -> anyhow::Result<()> {
    let target = updates.entry(sheet.to_string()).or_default();
    for (column_offset, column) in columns.iter().enumerate() {
        let column_offset =
            u32::try_from(column_offset).context("spreadsheet column offset is too large")?;
        let address_column = start
            .column
            .checked_add(column_offset)
            .context("spreadsheet destination column overflow")?;
        for (row_offset, value) in column.iter().enumerate() {
            let row_offset =
                u32::try_from(row_offset).context("spreadsheet row offset is too large")?;
            target.push(CellUpdate {
                address: CellAddress {
                    row: start
                        .row
                        .checked_add(row_offset)
                        .context("spreadsheet destination row overflow")?,
                    column: address_column,
                },
                value: value.clone(),
                style_from: None,
            });
        }
    }
    Ok(())
}

fn spreadsheet_cell_to_input(
    cell: &SpreadsheetCell,
    content_mode: SpreadsheetCopyContentMode,
) -> SpreadsheetCellInput {
    if matches!(content_mode, SpreadsheetCopyContentMode::ValuesAndFormulas) {
        if let Some(expression) = &cell.formula {
            return SpreadsheetCellInput::Formula(FormulaInput {
                expression: expression.clone(),
                cached_result: spreadsheet_cell_cached_result(&cell.value),
            });
        }
    }
    match &cell.value {
        SpreadsheetCellValue::Empty => SpreadsheetCellInput::Blank,
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => SpreadsheetCellInput::String(value.clone()),
        SpreadsheetCellValue::Integer(value) => SpreadsheetCellInput::Integer(*value),
        SpreadsheetCellValue::Number(value) => SpreadsheetCellInput::Number(*value),
        SpreadsheetCellValue::Boolean(value) => SpreadsheetCellInput::Boolean(*value),
        SpreadsheetCellValue::DateTime(value) => SpreadsheetCellInput::Number(value.serial),
    }
}

fn spreadsheet_cell_is_blank(cell: &SpreadsheetCell) -> bool {
    cell.formula.is_none()
        && match &cell.value {
            SpreadsheetCellValue::Empty => true,
            SpreadsheetCellValue::String(value) => value.trim().is_empty(),
            _ => false,
        }
}

fn spreadsheet_cell_cached_result(value: &SpreadsheetCellValue) -> Option<String> {
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

fn spreadsheet_success_result(
    call_id: Uuid,
    result: SpreadsheetResult,
    changed_path: Option<PathBuf>,
) -> anyhow::Result<ToolResult> {
    let action = result.kind();
    let validation_passed = match &result {
        SpreadsheetResult::WorkbookValidated(result) => Some(result.validation_passed),
        _ => None,
    };
    let value = serde_json::to_value(&result)?;
    let output = serde_json::to_string_pretty(&value)?;
    let mut content = vec![ModelContentPart::json(value.clone())];
    let mut metadata = json!({
        "toolName": "spreadsheet",
        "action": action,
        "success": true
    });
    if let (Some(validation_passed), Some(object)) = (validation_passed, metadata.as_object_mut()) {
        object.insert("validationPassed".to_string(), json!(validation_passed));
    }
    if let Some(path) = changed_path {
        let mime_type = match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("csv") => "text/csv",
            Some("tsv") | Some("tab") => "text/tab-separated-values",
            _ => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        };
        content.push(ModelContentPart::resource(
            path.to_string_lossy(),
            Some(mime_type.to_string()),
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
        ));
        if let Some(object) = metadata.as_object_mut() {
            object.insert("changedPath".to_string(), json!(path));
        }
    }
    Ok(ToolResult {
        call_id,
        output,
        content,
        metadata,
    })
}

pub(super) fn spreadsheet_error_result(
    call_id: Uuid,
    error: crate::spreadsheet::SpreadsheetError,
) -> ToolResult {
    let info = error.info();
    ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&info).unwrap_or_else(|_| error.to_string()),
        content: vec![ModelContentPart::json(
            serde_json::to_value(&info).unwrap_or_else(|_| json!({ "message": error.to_string() })),
        )],
        metadata: json!({
            "toolName": "spreadsheet",
            "success": false,
            "errorCode": info.code,
            "error": info.message
        }),
    }
}

fn remap_spreadsheet_paths(
    result: &mut SpreadsheetResult,
    source: Option<&Path>,
    output: Option<&Path>,
) {
    match result {
        SpreadsheetResult::DelimitedInspected(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::WorkbookInspected(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::SheetsListed(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::RangeRead(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::RangesRead(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
                for range in &mut result.ranges {
                    range.path = source.to_path_buf();
                }
            }
        }
        SpreadsheetResult::CellsFound(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::RowsFiltered(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::WorkbookValidated(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::DelimitedExported(result) => {
            if let Some(source) = source {
                result.source = source.to_path_buf();
            }
            if let Some(output) = output {
                result.output = output.to_path_buf();
            }
        }
        SpreadsheetResult::WorkbookWritten(result) => {
            if let Some(output) = output {
                result.output = output.to_path_buf();
            }
        }
    }
}

fn ensure_workbook_path(path: &Path) -> anyhow::Result<SpreadsheetFileFormat> {
    let format = SpreadsheetFileFormat::from_path(path).with_context(|| {
        format!(
            "unsupported spreadsheet source {}; expected one of {}",
            path.display(),
            SpreadsheetFileFormat::ATTACHMENT_EXTENSIONS.join(", ")
        )
    })?;
    anyhow::ensure!(
        format.is_workbook(),
        "spreadsheet workbook actions require a workbook file; use delimited actions for {}",
        path.display()
    );
    Ok(format)
}

fn staging_file_name(prefix: &str, path: &Path, fallback_extension: &str) -> String {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .unwrap_or(fallback_extension);
    format!("{prefix}.{extension}")
}

fn resolve_delimited_format(
    path: &Path,
    format: Option<DelimitedFormat>,
) -> anyhow::Result<DelimitedFormat> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let inferred = match extension.as_deref() {
        Some("csv") => Some(DelimitedFormat::Csv),
        Some("tsv") | Some("tab") => Some(DelimitedFormat::Tsv),
        _ => None,
    };
    match (format, inferred) {
        (Some(requested), Some(inferred)) if requested != inferred => anyhow::bail!(
            "spreadsheet delimited format {:?} does not match {}",
            requested,
            path.display()
        ),
        (Some(requested), _) => Ok(requested),
        (None, Some(inferred)) => Ok(inferred),
        (None, None) => anyhow::bail!(
            "spreadsheet delimited actions require a .csv, .tsv, or .tab path, or an explicit format"
        ),
    }
}

struct SpreadsheetStaging {
    root: PathBuf,
}

impl SpreadsheetStaging {
    fn new() -> anyhow::Result<Self> {
        let root = std::env::temp_dir().join(format!("opentopia-xlsx-{}", Uuid::new_v4()));
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for SpreadsheetStaging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
