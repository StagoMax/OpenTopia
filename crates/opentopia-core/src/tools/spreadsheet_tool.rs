use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, enforce_read_policy,
    insert_attachment_provenance, normalize_workspace_path, read_stored_attachment_file,
    required_typed_string, tool_resource_key, Tool, ToolExecutionPolicy, ToolInvocationContext,
    ToolSideEffect, TypedTool,
};
use crate::execution::{FileReadRequest, FileWriteRequest};
use crate::execution_authorization::ToolExecutionIntent;
use crate::file_mutation::read_optional;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::PolicyDecision;
use crate::spreadsheet::{
    execute_spreadsheet, CellAddress, CellRange, CellUpdate, DelimitedColumnMapping,
    DelimitedFormat, DelimitedFormulaMode, ExportDelimitedRequest, FillTemplateRequest,
    FilterRowsRequest, FindCellsRequest, FormulaInput, InspectDelimitedRequest,
    InspectWorkbookRequest, ListSheetsRequest, ReadRangeRequest, ReadRangesRequest,
    SheetRangeRequest, SheetWriteRequest, SpreadsheetAction, SpreadsheetCell, SpreadsheetCellInput,
    SpreadsheetCellValue, SpreadsheetFileFormat, SpreadsheetFilterCondition,
    SpreadsheetFilterMatchMode, SpreadsheetRequest, SpreadsheetResult, SpreadsheetSheetValidation,
    SpreadsheetTextMatchMode, ValidateWorkbookRequest, WriteWorkbookRequest,
    MAX_INPUT_FILE_BYTES as MAX_SPREADSHEET_INPUT_BYTES,
};
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
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
    FillTemplate,
    ExportDelimited,
    Write,
    WriteRows,
    WriteColumns,
    CopyRows,
    CopyColumns,
    Batch,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum SpreadsheetCopyContentMode {
    #[default]
    Values,
    ValuesAndFormulas,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum SpreadsheetBatchOperation {
    #[schemars(rename_all = "camelCase")]
    WriteRows {
        sheet: String,
        start: CellAddress,
        #[schemars(length(max = 10000))]
        rows: Vec<Vec<SpreadsheetCellInput>>,
    },
    #[schemars(rename_all = "camelCase")]
    WriteColumns {
        sheet: String,
        start: CellAddress,
        #[schemars(length(max = 256))]
        columns: Vec<Vec<SpreadsheetCellInput>>,
    },
    #[schemars(rename_all = "camelCase")]
    CopyRows {
        source_path: String,
        source_sheet: String,
        source_start: CellAddress,
        row_count: u32,
        column_count: u32,
        destination_sheet: String,
        destination_start: CellAddress,
        #[serde(default)]
        content_mode: SpreadsheetCopyContentMode,
    },
    #[schemars(rename_all = "camelCase")]
    CopyColumns {
        source_path: String,
        source_sheet: String,
        source_start: CellAddress,
        row_count: u32,
        column_count: u32,
        destination_sheet: String,
        destination_start: CellAddress,
        #[serde(default)]
        content_mode: SpreadsheetCopyContentMode,
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
        /// CSV/TSV path. Provide exactly one of path or attachmentId.
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
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
        /// Workspace-relative workbook path (.xls/.xlsx/.xlsm/.xlsb/.xltx/.xltm/.ods). Provide exactly one of path or attachmentId.
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
    },
    #[schemars(rename_all = "camelCase")]
    ListSheets {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
    },
    #[schemars(rename_all = "camelCase")]
    ReadRange {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
        sheet: String,
        /// Inclusive zero-based range.
        range: CellRange,
    },
    #[schemars(rename_all = "camelCase")]
    ReadRanges {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
        #[schemars(length(min = 1, max = 64))]
        ranges: Vec<SheetRangeRequest>,
    },
    #[schemars(rename_all = "camelCase")]
    ReadRows {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
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
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
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
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
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
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
        sheet: String,
        range: CellRange,
        #[schemars(length(min = 1, max = 32))]
        conditions: Vec<SpreadsheetFilterCondition>,
        #[serde(default)]
        filter_match_mode: Option<SpreadsheetFilterMatchMode>,
        #[serde(default)]
        #[schemars(range(min = 1, max = 1000))]
        max_results: Option<usize>,
    },
    #[schemars(rename_all = "camelCase")]
    Validate {
        /// Workbook path (.xls/.xlsx/.xlsm/.xlsb/.xltx/.xltm/.ods). Provide exactly one of path or attachmentId.
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        attachment_id: Option<String>,
        #[serde(default)]
        expected_sheets: Vec<String>,
        #[serde(default)]
        expected_populated_cells: Option<u64>,
        #[serde(default)]
        sheets: Vec<SpreadsheetSheetValidation>,
    },
    #[schemars(rename_all = "camelCase")]
    FillTemplate {
        /// CSV/TSV source data path.
        data_path: String,
        /// Existing XLSX template path.
        template_path: String,
        /// Destination XLSX path.
        output_path: String,
        target_sheet: String,
        #[serde(default)]
        source_format: Option<DelimitedFormat>,
        #[serde(default)]
        source_header_row: u32,
        #[serde(default)]
        target_header_row: u32,
        #[serde(default)]
        target_start_row: Option<u32>,
        /// Omit when equal header names can be matched automatically.
        #[serde(default)]
        mappings: Vec<DelimitedColumnMapping>,
        #[serde(default)]
        rstrip_tabs: bool,
    },
    #[schemars(rename_all = "camelCase")]
    ExportDelimited {
        /// Source workbook path.
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
        /// Workspace-relative destination selected by the model from the user's request.
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        source_path: Option<String>,
        #[serde(default)]
        output_path: Option<String>,
        #[schemars(length(max = 256))]
        sheets: Vec<SheetWriteRequest>,
    },
    #[schemars(rename_all = "camelCase")]
    WriteRows {
        #[serde(default)]
        path: Option<String>,
        sheet: String,
        start: CellAddress,
        #[schemars(length(min = 1, max = 10000))]
        rows: Vec<Vec<SpreadsheetCellInput>>,
        #[serde(default)]
        source_path: Option<String>,
        #[serde(default)]
        output_path: Option<String>,
        #[serde(default)]
        atomic: Option<bool>,
    },
    #[schemars(rename_all = "camelCase")]
    WriteColumns {
        #[serde(default)]
        path: Option<String>,
        sheet: String,
        start: CellAddress,
        #[schemars(length(min = 1, max = 256))]
        columns: Vec<Vec<SpreadsheetCellInput>>,
        #[serde(default)]
        source_path: Option<String>,
        #[serde(default)]
        output_path: Option<String>,
        #[serde(default)]
        atomic: Option<bool>,
    },
    #[schemars(rename_all = "camelCase")]
    CopyRows {
        #[serde(default)]
        path: Option<String>,
        from: String,
        source_sheet: String,
        source_start: CellAddress,
        #[schemars(range(min = 1))]
        row_count: u32,
        #[schemars(range(min = 1))]
        column_count: u32,
        destination_sheet: String,
        destination_start: CellAddress,
        #[serde(default)]
        content_mode: SpreadsheetCopyContentMode,
        #[serde(default)]
        source_path: Option<String>,
        #[serde(default)]
        output_path: Option<String>,
        #[serde(default)]
        atomic: Option<bool>,
    },
    #[schemars(rename_all = "camelCase")]
    CopyColumns {
        #[serde(default)]
        path: Option<String>,
        from: String,
        source_sheet: String,
        source_start: CellAddress,
        #[schemars(range(min = 1))]
        row_count: u32,
        #[schemars(range(min = 1))]
        column_count: u32,
        destination_sheet: String,
        destination_start: CellAddress,
        #[serde(default)]
        content_mode: SpreadsheetCopyContentMode,
        #[serde(default)]
        source_path: Option<String>,
        #[serde(default)]
        output_path: Option<String>,
        #[serde(default)]
        atomic: Option<bool>,
    },
    #[schemars(rename_all = "camelCase")]
    Batch {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        source_path: Option<String>,
        #[serde(default)]
        output_path: Option<String>,
        /// Ordered operations validated before the output is written.
        #[schemars(length(min = 1, max = 64))]
        operations: Vec<SpreadsheetBatchOperation>,
        /// Mutations are validate-then-write and atomic. Omit or set true.
        #[serde(default)]
        atomic: Option<bool>,
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
            Self::FillTemplate { .. } => SpreadsheetToolAction::FillTemplate,
            Self::ExportDelimited { .. } => SpreadsheetToolAction::ExportDelimited,
            Self::Write { .. } => SpreadsheetToolAction::Write,
            Self::WriteRows { .. } => SpreadsheetToolAction::WriteRows,
            Self::WriteColumns { .. } => SpreadsheetToolAction::WriteColumns,
            Self::CopyRows { .. } => SpreadsheetToolAction::CopyRows,
            Self::CopyColumns { .. } => SpreadsheetToolAction::CopyColumns,
            Self::Batch { .. } => SpreadsheetToolAction::Batch,
        }
    }

    fn path(&self) -> Option<&str> {
        match self {
            Self::InspectDelimited { path, .. }
            | Self::Inspect { path, .. }
            | Self::ListSheets { path, .. }
            | Self::ReadRange { path, .. }
            | Self::ReadRanges { path, .. }
            | Self::ReadRows { path, .. }
            | Self::ReadColumns { path, .. }
            | Self::Find { path, .. }
            | Self::FilterRows { path, .. }
            | Self::Validate { path, .. }
            | Self::Write { path, .. }
            | Self::WriteRows { path, .. }
            | Self::WriteColumns { path, .. }
            | Self::CopyRows { path, .. }
            | Self::CopyColumns { path, .. }
            | Self::Batch { path, .. } => path.as_deref(),
            Self::ExportDelimited { path, .. } => Some(path.as_str()),
            Self::FillTemplate { .. } => None,
        }
    }

    fn attachment_id(&self) -> Option<&str> {
        match self {
            Self::InspectDelimited { attachment_id, .. }
            | Self::Inspect { attachment_id, .. }
            | Self::ListSheets { attachment_id, .. }
            | Self::ReadRange { attachment_id, .. }
            | Self::ReadRanges { attachment_id, .. }
            | Self::ReadRows { attachment_id, .. }
            | Self::ReadColumns { attachment_id, .. }
            | Self::Find { attachment_id, .. }
            | Self::FilterRows { attachment_id, .. }
            | Self::Validate { attachment_id, .. } => attachment_id.as_deref(),
            Self::FillTemplate { .. }
            | Self::ExportDelimited { .. }
            | Self::Write { .. }
            | Self::WriteRows { .. }
            | Self::WriteColumns { .. }
            | Self::CopyRows { .. }
            | Self::CopyColumns { .. }
            | Self::Batch { .. } => None,
        }
    }

    fn mutation_paths(&self) -> Vec<&str> {
        let mut paths = Vec::new();
        match self {
            Self::FillTemplate {
                data_path,
                template_path,
                output_path,
                ..
            } => paths.extend([
                data_path.as_str(),
                template_path.as_str(),
                output_path.as_str(),
            ]),
            Self::ExportDelimited {
                path, output_path, ..
            } => paths.extend([path.as_str(), output_path.as_str()]),
            Self::Write {
                path,
                source_path,
                output_path,
                ..
            }
            | Self::WriteRows {
                path,
                source_path,
                output_path,
                ..
            }
            | Self::WriteColumns {
                path,
                source_path,
                output_path,
                ..
            }
            | Self::Batch {
                path,
                source_path,
                output_path,
                ..
            } => paths.extend(
                path.iter()
                    .chain(source_path.iter())
                    .chain(output_path.iter())
                    .map(String::as_str),
            ),
            Self::CopyRows {
                path,
                from,
                source_path,
                output_path,
                ..
            }
            | Self::CopyColumns {
                path,
                from,
                source_path,
                output_path,
                ..
            } => {
                paths.extend(
                    path.iter()
                        .chain(source_path.iter())
                        .chain(output_path.iter())
                        .map(String::as_str),
                );
                paths.push(from);
            }
            Self::InspectDelimited { .. }
            | Self::Inspect { .. }
            | Self::ListSheets { .. }
            | Self::ReadRange { .. }
            | Self::ReadRanges { .. }
            | Self::ReadRows { .. }
            | Self::ReadColumns { .. }
            | Self::Find { .. }
            | Self::FilterRows { .. }
            | Self::Validate { .. } => {}
        }
        if let Self::Batch { operations, .. } = self {
            paths.extend(
                operations
                    .iter()
                    .filter_map(spreadsheet_operation_source_path),
            );
        }
        paths
    }

    fn mutation_output_path(&self) -> Option<&str> {
        match self {
            Self::FillTemplate { output_path, .. } | Self::ExportDelimited { output_path, .. } => {
                Some(output_path.as_str())
            }
            Self::Write {
                path, output_path, ..
            }
            | Self::WriteRows {
                path, output_path, ..
            }
            | Self::WriteColumns {
                path, output_path, ..
            }
            | Self::CopyRows {
                path, output_path, ..
            }
            | Self::CopyColumns {
                path, output_path, ..
            }
            | Self::Batch {
                path, output_path, ..
            } => output_path.as_deref().or(path.as_deref()),
            Self::InspectDelimited { .. }
            | Self::Inspect { .. }
            | Self::ListSheets { .. }
            | Self::ReadRange { .. }
            | Self::ReadRanges { .. }
            | Self::ReadRows { .. }
            | Self::ReadColumns { .. }
            | Self::Find { .. }
            | Self::FilterRows { .. }
            | Self::Validate { .. } => None,
        }
    }

    fn mutation_read_paths(&self) -> Vec<&str> {
        let mut paths = Vec::new();
        match self {
            Self::FillTemplate {
                data_path,
                template_path,
                ..
            } => paths.extend([data_path.as_str(), template_path.as_str()]),
            Self::ExportDelimited { path, .. } => paths.push(path.as_str()),
            Self::Write {
                path, source_path, ..
            }
            | Self::WriteRows {
                path, source_path, ..
            }
            | Self::WriteColumns {
                path, source_path, ..
            }
            | Self::Batch {
                path, source_path, ..
            } => paths.extend(source_path.iter().chain(path.iter()).map(String::as_str)),
            Self::CopyRows {
                path,
                from,
                source_path,
                ..
            }
            | Self::CopyColumns {
                path,
                from,
                source_path,
                ..
            } => {
                paths.extend(source_path.iter().chain(path.iter()).map(String::as_str));
                paths.push(from);
            }
            Self::InspectDelimited { .. }
            | Self::Inspect { .. }
            | Self::ListSheets { .. }
            | Self::ReadRange { .. }
            | Self::ReadRanges { .. }
            | Self::ReadRows { .. }
            | Self::ReadColumns { .. }
            | Self::Find { .. }
            | Self::FilterRows { .. }
            | Self::Validate { .. } => {}
        }
        if let Self::Batch { operations, .. } = self {
            paths.extend(
                operations
                    .iter()
                    .filter_map(spreadsheet_operation_source_path),
            );
        }
        paths
    }

    fn into_execution_input(self) -> SpreadsheetExecutionInput {
        SpreadsheetExecutionInput::from(self)
    }
}

struct SpreadsheetExecutionInput {
    action: SpreadsheetToolAction,
    path: Option<String>,
    attachment_id: Option<String>,
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
    max_results: Option<usize>,
    start: Option<CellAddress>,
    rows: Vec<Vec<SpreadsheetCellInput>>,
    columns: Vec<Vec<SpreadsheetCellInput>>,
    from: Option<String>,
    source_sheet: Option<String>,
    source_start: Option<CellAddress>,
    destination_sheet: Option<String>,
    destination_start: Option<CellAddress>,
    content_mode: SpreadsheetCopyContentMode,
    source_path: Option<String>,
    output_path: Option<String>,
    data_path: Option<String>,
    template_path: Option<String>,
    target_header_row: u32,
    target_start_row: Option<u32>,
    mappings: Vec<DelimitedColumnMapping>,
    formula_mode: DelimitedFormulaMode,
    expected_sheets: Vec<String>,
    expected_populated_cells: Option<u64>,
    sheet_validations: Vec<SpreadsheetSheetValidation>,
    sheets: Vec<SheetWriteRequest>,
    operations: Vec<SpreadsheetBatchOperation>,
    atomic: Option<bool>,
}

impl SpreadsheetExecutionInput {
    fn mutation_output_path(&self) -> Option<&str> {
        self.output_path.as_deref().or(self.path.as_deref())
    }

    fn compact_direct_operation(&self) -> anyhow::Result<Option<SpreadsheetBatchOperation>> {
        let operation = match self.action {
            SpreadsheetToolAction::WriteRows => Some(SpreadsheetBatchOperation::WriteRows {
                sheet: required_typed_string(self.sheet.as_deref(), "sheet")?,
                start: self
                    .start
                    .context("spreadsheet write_rows requires start")?,
                rows: self.rows.clone(),
            }),
            SpreadsheetToolAction::WriteColumns => Some(SpreadsheetBatchOperation::WriteColumns {
                sheet: required_typed_string(self.sheet.as_deref(), "sheet")?,
                start: self
                    .start
                    .context("spreadsheet write_columns requires start")?,
                columns: self.columns.clone(),
            }),
            SpreadsheetToolAction::CopyRows | SpreadsheetToolAction::CopyColumns => {
                let source_path = required_typed_string(self.from.as_deref(), "from")?;
                let source_sheet =
                    required_typed_string(self.source_sheet.as_deref(), "sourceSheet")?;
                let source_start = self
                    .source_start
                    .context("spreadsheet copy requires sourceStart")?;
                let row_count = self
                    .row_count
                    .context("spreadsheet copy requires rowCount")?;
                let column_count = self
                    .column_count
                    .context("spreadsheet copy requires columnCount")?;
                let destination_sheet =
                    required_typed_string(self.destination_sheet.as_deref(), "destinationSheet")?;
                let destination_start = self
                    .destination_start
                    .context("spreadsheet copy requires destinationStart")?;
                Some(if self.action == SpreadsheetToolAction::CopyRows {
                    SpreadsheetBatchOperation::CopyRows {
                        source_path,
                        source_sheet,
                        source_start,
                        row_count,
                        column_count,
                        destination_sheet,
                        destination_start,
                        content_mode: self.content_mode,
                    }
                } else {
                    SpreadsheetBatchOperation::CopyColumns {
                        source_path,
                        source_sheet,
                        source_start,
                        row_count,
                        column_count,
                        destination_sheet,
                        destination_start,
                        content_mode: self.content_mode,
                    }
                })
            }
            SpreadsheetToolAction::InspectDelimited
            | SpreadsheetToolAction::Inspect
            | SpreadsheetToolAction::ListSheets
            | SpreadsheetToolAction::ReadRange
            | SpreadsheetToolAction::ReadRanges
            | SpreadsheetToolAction::ReadRows
            | SpreadsheetToolAction::ReadColumns
            | SpreadsheetToolAction::Find
            | SpreadsheetToolAction::FilterRows
            | SpreadsheetToolAction::Validate
            | SpreadsheetToolAction::FillTemplate
            | SpreadsheetToolAction::ExportDelimited
            | SpreadsheetToolAction::Write
            | SpreadsheetToolAction::Batch => None,
        };
        Ok(operation)
    }
}

impl From<SpreadsheetToolInput> for SpreadsheetExecutionInput {
    fn from(input: SpreadsheetToolInput) -> Self {
        let mut execution = Self {
            action: input.action(),
            path: None,
            attachment_id: None,
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
            max_results: None,
            start: None,
            rows: Vec::new(),
            columns: Vec::new(),
            from: None,
            source_sheet: None,
            source_start: None,
            destination_sheet: None,
            destination_start: None,
            content_mode: SpreadsheetCopyContentMode::Values,
            source_path: None,
            output_path: None,
            data_path: None,
            template_path: None,
            target_header_row: 0,
            target_start_row: None,
            mappings: Vec::new(),
            formula_mode: DelimitedFormulaMode::Values,
            expected_sheets: Vec::new(),
            expected_populated_cells: None,
            sheet_validations: Vec::new(),
            sheets: Vec::new(),
            operations: Vec::new(),
            atomic: None,
        };
        match input {
            SpreadsheetToolInput::InspectDelimited {
                path,
                attachment_id,
                format,
                header_row,
                sample_rows,
                rstrip_tabs,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
                execution.format = format;
                execution.header_row = header_row;
                execution.sample_rows = sample_rows;
                execution.rstrip_tabs = rstrip_tabs;
            }
            SpreadsheetToolInput::Inspect {
                path,
                attachment_id,
            }
            | SpreadsheetToolInput::ListSheets {
                path,
                attachment_id,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
            }
            SpreadsheetToolInput::ReadRange {
                path,
                attachment_id,
                sheet,
                range,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
                execution.sheet = Some(sheet);
                execution.range = Some(range);
            }
            SpreadsheetToolInput::ReadRanges {
                path,
                attachment_id,
                ranges,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
                execution.ranges = ranges;
            }
            SpreadsheetToolInput::ReadRows {
                path,
                attachment_id,
                sheet,
                start_row,
                start_column,
                row_count,
                column_count,
            }
            | SpreadsheetToolInput::ReadColumns {
                path,
                attachment_id,
                sheet,
                start_row,
                start_column,
                row_count,
                column_count,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
                execution.sheet = Some(sheet);
                execution.start_row = Some(start_row);
                execution.start_column = Some(start_column);
                execution.row_count = Some(row_count);
                execution.column_count = Some(column_count);
            }
            SpreadsheetToolInput::Find {
                path,
                attachment_id,
                sheet,
                range,
                query,
                match_mode,
                case_sensitive,
                include_formulas,
                max_results,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
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
                attachment_id,
                sheet,
                range,
                conditions,
                filter_match_mode,
                max_results,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
                execution.sheet = Some(sheet);
                execution.range = Some(range);
                execution.conditions = conditions;
                execution.filter_match_mode = filter_match_mode;
                execution.max_results = max_results;
            }
            SpreadsheetToolInput::Validate {
                path,
                attachment_id,
                expected_sheets,
                expected_populated_cells,
                sheets,
            } => {
                execution.path = path;
                execution.attachment_id = attachment_id;
                execution.expected_sheets = expected_sheets;
                execution.expected_populated_cells = expected_populated_cells;
                execution.sheet_validations = sheets;
            }
            SpreadsheetToolInput::FillTemplate {
                data_path,
                template_path,
                output_path,
                target_sheet,
                source_format,
                source_header_row,
                target_header_row,
                target_start_row,
                mappings,
                rstrip_tabs,
            } => {
                execution.data_path = Some(data_path);
                execution.template_path = Some(template_path);
                execution.output_path = Some(output_path);
                execution.sheet = Some(target_sheet);
                execution.format = source_format;
                execution.header_row = source_header_row;
                execution.target_header_row = target_header_row;
                execution.target_start_row = target_start_row;
                execution.mappings = mappings;
                execution.rstrip_tabs = rstrip_tabs;
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
                source_path,
                output_path,
                sheets,
            } => {
                execution.path = path;
                execution.source_path = source_path;
                execution.output_path = output_path;
                execution.sheets = sheets;
            }
            SpreadsheetToolInput::WriteRows {
                path,
                sheet,
                start,
                rows,
                source_path,
                output_path,
                atomic,
            } => {
                execution.path = path;
                execution.sheet = Some(sheet);
                execution.start = Some(start);
                execution.rows = rows;
                execution.source_path = source_path;
                execution.output_path = output_path;
                execution.atomic = atomic;
            }
            SpreadsheetToolInput::WriteColumns {
                path,
                sheet,
                start,
                columns,
                source_path,
                output_path,
                atomic,
            } => {
                execution.path = path;
                execution.sheet = Some(sheet);
                execution.start = Some(start);
                execution.columns = columns;
                execution.source_path = source_path;
                execution.output_path = output_path;
                execution.atomic = atomic;
            }
            SpreadsheetToolInput::CopyRows {
                path,
                from,
                source_sheet,
                source_start,
                row_count,
                column_count,
                destination_sheet,
                destination_start,
                content_mode,
                source_path,
                output_path,
                atomic,
            }
            | SpreadsheetToolInput::CopyColumns {
                path,
                from,
                source_sheet,
                source_start,
                row_count,
                column_count,
                destination_sheet,
                destination_start,
                content_mode,
                source_path,
                output_path,
                atomic,
            } => {
                execution.path = path;
                execution.from = Some(from);
                execution.source_sheet = Some(source_sheet);
                execution.source_start = Some(source_start);
                execution.row_count = Some(row_count);
                execution.column_count = Some(column_count);
                execution.destination_sheet = Some(destination_sheet);
                execution.destination_start = Some(destination_start);
                execution.content_mode = content_mode;
                execution.source_path = source_path;
                execution.output_path = output_path;
                execution.atomic = atomic;
            }
            SpreadsheetToolInput::Batch {
                path,
                source_path,
                output_path,
                operations,
                atomic,
            } => {
                execution.path = path;
                execution.source_path = source_path;
                execution.output_path = output_path;
                execution.operations = operations;
                execution.atomic = atomic;
            }
        }
        execution
    }
}

pub struct SpreadsheetTool;

#[async_trait]
impl TypedTool for SpreadsheetTool {
    type Input = SpreadsheetToolInput;

    fn name(&self) -> &str {
        "spreadsheet"
    }

    fn description(&self) -> &str {
        "Inspect common workbook and delimited-data files, then execute bounded reads or atomic writes to the destination selected by the model. CSV quoting, embedded newlines, duplicate headers, and optional trailing-tab cleanup are handled server-side."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        match input.action() {
            SpreadsheetToolAction::InspectDelimited
            | SpreadsheetToolAction::Inspect
            | SpreadsheetToolAction::ListSheets
            | SpreadsheetToolAction::ReadRange
            | SpreadsheetToolAction::ReadRanges
            | SpreadsheetToolAction::ReadRows
            | SpreadsheetToolAction::ReadColumns
            | SpreadsheetToolAction::Find
            | SpreadsheetToolAction::FilterRows
            | SpreadsheetToolAction::Validate => {
                ToolExecutionPolicy::read_only(vec![tool_resource_key(
                    if input.attachment_id().is_some() {
                        "attachment"
                    } else {
                        "file"
                    },
                    input.attachment_id().or(input.path()).unwrap_or("*"),
                )])
            }
            SpreadsheetToolAction::FillTemplate
            | SpreadsheetToolAction::ExportDelimited
            | SpreadsheetToolAction::Write
            | SpreadsheetToolAction::WriteRows
            | SpreadsheetToolAction::WriteColumns
            | SpreadsheetToolAction::CopyRows
            | SpreadsheetToolAction::CopyColumns
            | SpreadsheetToolAction::Batch => {
                let mut resource_keys = input
                    .mutation_paths()
                    .into_iter()
                    .map(|path| tool_resource_key("file", path))
                    .collect::<Vec<_>>();
                resource_keys.sort();
                resource_keys.dedup();
                if resource_keys.is_empty() {
                    resource_keys.push("*".to_string());
                }
                ToolExecutionPolicy {
                    read_only: false,
                    idempotent: false,
                    parallel_safe: true,
                    side_effect: ToolSideEffect::WorkspaceWrite,
                    resource_keys,
                }
            }
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        match input.action() {
            SpreadsheetToolAction::InspectDelimited
            | SpreadsheetToolAction::Inspect
            | SpreadsheetToolAction::ListSheets
            | SpreadsheetToolAction::ReadRange
            | SpreadsheetToolAction::ReadRanges
            | SpreadsheetToolAction::ReadRows
            | SpreadsheetToolAction::ReadColumns
            | SpreadsheetToolAction::Find
            | SpreadsheetToolAction::FilterRows
            | SpreadsheetToolAction::Validate => {
                ToolExecutionIntent::observation(input.path().map(PathBuf::from))
            }
            SpreadsheetToolAction::FillTemplate
            | SpreadsheetToolAction::ExportDelimited
            | SpreadsheetToolAction::Write
            | SpreadsheetToolAction::WriteRows
            | SpreadsheetToolAction::WriteColumns
            | SpreadsheetToolAction::CopyRows
            | SpreadsheetToolAction::CopyColumns
            | SpreadsheetToolAction::Batch => {
                let read_paths = input.mutation_read_paths().into_iter().map(PathBuf::from);
                ToolExecutionIntent::workspace_mutation(
                    input.mutation_output_path().map(PathBuf::from),
                )
                .with_read_paths(read_paths)
            }
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
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
            | SpreadsheetToolAction::Validate => {
                execute_spreadsheet_read(call_id, input, ctx).await
            }
            SpreadsheetToolAction::FillTemplate => {
                execute_spreadsheet_fill_template(call_id, input, ctx).await
            }
            SpreadsheetToolAction::ExportDelimited => {
                execute_spreadsheet_export_delimited(call_id, input, ctx).await
            }
            SpreadsheetToolAction::Write => execute_spreadsheet_write(call_id, input, ctx).await,
            SpreadsheetToolAction::WriteRows
            | SpreadsheetToolAction::WriteColumns
            | SpreadsheetToolAction::CopyRows
            | SpreadsheetToolAction::CopyColumns
            | SpreadsheetToolAction::Batch => {
                execute_spreadsheet_mutations(call_id, input, ctx).await
            }
        }
    }
}

impl_typed_tool!(SpreadsheetTool);

fn spreadsheet_operation_source_path(operation: &SpreadsheetBatchOperation) -> Option<&str> {
    match operation {
        SpreadsheetBatchOperation::CopyRows { source_path, .. }
        | SpreadsheetBatchOperation::CopyColumns { source_path, .. } => Some(source_path),
        SpreadsheetBatchOperation::WriteRows { .. }
        | SpreadsheetBatchOperation::WriteColumns { .. } => None,
    }
}

async fn execute_spreadsheet_read(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let mut action = input.action;
    let mut reads_delimited = action == SpreadsheetToolAction::InspectDelimited;
    let path = input
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty());
    let attachment_id = input
        .attachment_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    anyhow::ensure!(
        path.is_some() ^ attachment_id.is_some(),
        "spreadsheet read action requires exactly one of path or attachmentId"
    );
    let (resolved_path, source_bytes, attachment_metadata, resolved_delimited_format) =
        if let Some(relative) = path {
            let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
            enforce_read_policy(&ctx, &logical_path)?;
            let resolved_path = ctx.environment.resolve_read_path(&logical_path)?;
            if action == SpreadsheetToolAction::Inspect
                && SpreadsheetFileFormat::from_path(&resolved_path)
                    .is_some_and(SpreadsheetFileFormat::is_delimited)
            {
                action = SpreadsheetToolAction::InspectDelimited;
                reads_delimited = true;
            }
            let resolved_format = if reads_delimited {
                Some(resolve_delimited_format(&resolved_path, input.format)?)
            } else {
                ensure_workbook_path(&resolved_path)?;
                None
            };
            let read = ctx
                .environment
                .read_file(
                    FileReadRequest::new(&resolved_path)
                        .with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES),
                )
                .await?;
            (read.path, read.bytes, None, resolved_format)
        } else {
            let attachment_id = Uuid::parse_str(attachment_id.expect("attachment id present"))
                .context("attachmentId must be a UUID from the attachment manifest")?;
            let attachment =
                read_stored_attachment_file(&ctx, attachment_id, MAX_SPREADSHEET_INPUT_BYTES)
                    .await?;
            let fallback_extension = if reads_delimited {
                input.format.unwrap_or_default().extension()
            } else {
                "xlsx"
            };
            let logical_path = attachment.original_logical_path(fallback_extension);
            if action == SpreadsheetToolAction::Inspect
                && SpreadsheetFileFormat::from_path(&logical_path)
                    .is_some_and(SpreadsheetFileFormat::is_delimited)
            {
                action = SpreadsheetToolAction::InspectDelimited;
                reads_delimited = true;
            }
            let resolved_format = if reads_delimited {
                Some(resolve_delimited_format(&logical_path, input.format)?)
            } else {
                ensure_workbook_path(&logical_path)?;
                None
            };
            let metadata = attachment.metadata();
            (
                logical_path,
                attachment.data,
                Some(metadata),
                resolved_format,
            )
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
            SpreadsheetToolAction::FillTemplate
            | SpreadsheetToolAction::ExportDelimited
            | SpreadsheetToolAction::Write
            | SpreadsheetToolAction::WriteRows
            | SpreadsheetToolAction::WriteColumns
            | SpreadsheetToolAction::CopyRows
            | SpreadsheetToolAction::CopyColumns
            | SpreadsheetToolAction::Batch => unreachable!(),
        };
        Ok(execute_spreadsheet(SpreadsheetRequest { action }))
    })
    .await
    .context("spreadsheet worker task failed")??;
    let mut result = match outcome {
        Ok(result) => result,
        Err(error) => {
            let mut result = spreadsheet_error_result(call_id, error);
            if let Some(metadata) = attachment_metadata.as_ref() {
                insert_attachment_provenance(&mut result.metadata, metadata);
            }
            return Ok(result);
        }
    };
    remap_spreadsheet_paths(&mut result, Some(&resolved_path), None);
    let mut result = spreadsheet_success_result(call_id, result, None)?;
    if let Some(metadata) = attachment_metadata {
        insert_attachment_provenance(&mut result.metadata, &metadata);
    }
    Ok(result)
}

async fn execute_spreadsheet_fill_template(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let data_relative = required_typed_string(input.data_path.as_deref(), "dataPath")?;
    let template_relative = required_typed_string(input.template_path.as_deref(), "templatePath")?;
    let output_relative = required_typed_string(input.output_path.as_deref(), "outputPath")?;
    let data_logical = normalize_workspace_path(&ctx.workspace_root, &data_relative)?;
    let template_logical = normalize_workspace_path(&ctx.workspace_root, &template_relative)?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, &output_relative)?;
    enforce_read_policy(&ctx, &data_logical)?;
    enforce_read_policy(&ctx, &template_logical)?;
    let source_format = resolve_delimited_format(&data_logical, input.format)?;
    ensure_xlsx_path(&template_logical)?;
    ensure_xlsx_path(&output_path)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), &ctx)?;

    let data_path = ctx.environment.resolve_read_path(&data_logical)?;
    let template_path = ctx.environment.resolve_read_path(&template_logical)?;
    let data = ctx
        .environment
        .read_file(FileReadRequest::new(&data_path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
        .await?;
    let template = ctx
        .environment
        .read_file(FileReadRequest::new(&template_path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
        .await?;
    let source_format = Some(source_format);
    let target_sheet = required_typed_string(input.sheet.as_deref(), "targetSheet")?;
    let source_header_row = input.header_row;
    let target_header_row = input.target_header_row;
    let target_start_row = input.target_start_row;
    let mappings = input.mappings;
    let rstrip_tabs = input.rstrip_tabs;
    let staged = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_data = staging.path(match source_format.unwrap_or_default() {
            DelimitedFormat::Csv => "source.csv",
            DelimitedFormat::Tsv => "source.tsv",
        });
        let staged_template = staging.path("template.xlsx");
        let staged_output = staging.path("output.xlsx");
        fs::write(&staged_data, data.bytes)
            .with_context(|| format!("failed to stage {}", data.path.display()))?;
        fs::write(&staged_template, template.bytes)
            .with_context(|| format!("failed to stage {}", template.path.display()))?;
        let outcome = execute_spreadsheet(SpreadsheetRequest {
            action: SpreadsheetAction::FillTemplate(FillTemplateRequest {
                source: staged_data,
                source_format,
                template: staged_template,
                output: staged_output.clone(),
                target_sheet,
                source_header_row,
                target_header_row,
                target_start_row,
                mappings,
                rstrip_tabs,
            }),
        });
        match outcome {
            Ok(result) => {
                let bytes = fs::read(&staged_output)
                    .with_context(|| format!("failed to read {}", staged_output.display()))?;
                Ok(Ok((result, bytes, data.path, template.path)))
            }
            Err(error) => Ok(Err(error)),
        }
    })
    .await
    .context("spreadsheet fill_template worker task failed")??;
    let (mut result, bytes, resolved_data_path, resolved_template_path) = match staged {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let written = ctx
        .environment
        .write_file(FileWriteRequest::new(&output_path, bytes))
        .await?;
    if let SpreadsheetResult::TemplateFilled(filled) = &mut result {
        filled.source = resolved_data_path;
        filled.template = resolved_template_path;
    }
    remap_spreadsheet_paths(&mut result, None, Some(&written.path));
    spreadsheet_success_result(call_id, result, Some(written.path))
}

async fn execute_spreadsheet_export_delimited(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let source_relative = required_typed_string(input.path.as_deref(), "path")?;
    let output_relative = required_typed_string(input.output_path.as_deref(), "outputPath")?;
    let source_logical = normalize_workspace_path(&ctx.workspace_root, &source_relative)?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, &output_relative)?;
    enforce_read_policy(&ctx, &source_logical)?;
    ensure_workbook_path(&source_logical)?;
    let format = resolve_delimited_format(&output_path, input.format)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), &ctx)?;

    let resolved_source = ctx.environment.resolve_read_path(&source_logical)?;
    let source = ctx
        .environment
        .read_file(
            FileReadRequest::new(&resolved_source).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES),
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
    let written = ctx
        .environment
        .write_file(FileWriteRequest::new(&output_path, bytes))
        .await?;
    remap_spreadsheet_paths(
        &mut result,
        Some(&resolved_source_path),
        Some(&written.path),
    );
    spreadsheet_success_result(call_id, result, Some(written.path))
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

    let source = if let Some(relative) = input
        .source_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;
        ensure_spreadsheet_source_path(&path)?;
        Some(
            ctx.environment
                .read_file(FileReadRequest::new(&path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
                .await?,
        )
    } else {
        enforce_read_policy(&ctx, &output_path)?;
        read_optional(ctx.environment.as_ref(), &output_path)
            .await?
            .map(|bytes| crate::execution::FileReadResult {
                path: output_path.clone(),
                bytes,
            })
    };

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
    let written = ctx
        .environment
        .write_file(FileWriteRequest::new(&output_path, bytes))
        .await?;
    remap_spreadsheet_paths(&mut result, None, Some(&written.path));
    spreadsheet_success_result(call_id, result, Some(written.path))
}

async fn execute_spreadsheet_mutations(
    call_id: Uuid,
    input: SpreadsheetExecutionInput,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    anyhow::ensure!(
        input.atomic.unwrap_or(true),
        "spreadsheet mutations are always atomic; atomic=false is not supported"
    );
    let output_relative = input
        .mutation_output_path()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("spreadsheet mutation requires path")?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, output_relative)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), &ctx)?;

    let compact_operation = input.compact_direct_operation()?;
    let operations =
        spreadsheet_operations_for_action(input.action, compact_operation, input.operations)?;
    let base_source = if let Some(relative) = input
        .source_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;
        ensure_spreadsheet_source_path(&path)?;
        Some(
            ctx.environment
                .read_file(FileReadRequest::new(&path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
                .await?,
        )
    } else {
        enforce_read_policy(&ctx, &output_path)?;
        read_optional(ctx.environment.as_ref(), &output_path)
            .await?
            .map(|bytes| crate::execution::FileReadResult {
                path: output_path.clone(),
                bytes,
            })
    };

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
        let sheets = materialize_spreadsheet_operations(&operations, &staged_copy_sources)?;
        let output_name = staging_file_name("output", &staged_output_format_path, "xlsx");
        let staged_output = staging.path(&output_name);
        let outcome = execute_spreadsheet(SpreadsheetRequest {
            action: SpreadsheetAction::WriteWorkbook(WriteWorkbookRequest {
                source: staged_base,
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
    .context("spreadsheet mutation worker task failed")??;
    let (mut result, bytes) = match staged {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let written = ctx
        .environment
        .write_file(FileWriteRequest::new(&output_path, bytes))
        .await?;
    remap_spreadsheet_paths(&mut result, None, Some(&written.path));
    spreadsheet_success_result(call_id, result, Some(written.path))
}

fn spreadsheet_operations_for_action(
    action: SpreadsheetToolAction,
    operation: Option<SpreadsheetBatchOperation>,
    operations: Vec<SpreadsheetBatchOperation>,
) -> anyhow::Result<Vec<SpreadsheetBatchOperation>> {
    if action == SpreadsheetToolAction::Batch {
        anyhow::ensure!(
            operation.is_none(),
            "spreadsheet batch uses operations, not operation"
        );
        anyhow::ensure!(
            !operations.is_empty(),
            "spreadsheet batch requires operations"
        );
        return Ok(operations);
    }
    anyhow::ensure!(
        operations.is_empty(),
        "direct spreadsheet mutations use operation, not operations"
    );
    let operation = operation.context("spreadsheet mutation requires operation")?;
    let matches_action = matches!(
        (&action, &operation),
        (
            SpreadsheetToolAction::WriteRows,
            SpreadsheetBatchOperation::WriteRows { .. }
        ) | (
            SpreadsheetToolAction::WriteColumns,
            SpreadsheetBatchOperation::WriteColumns { .. }
        ) | (
            SpreadsheetToolAction::CopyRows,
            SpreadsheetBatchOperation::CopyRows { .. }
        ) | (
            SpreadsheetToolAction::CopyColumns,
            SpreadsheetBatchOperation::CopyColumns { .. }
        )
    );
    anyhow::ensure!(
        matches_action,
        "spreadsheet operation type must match action"
    );
    Ok(vec![operation])
}

fn materialize_spreadsheet_operations(
    operations: &[SpreadsheetBatchOperation],
    copy_sources: &BTreeMap<String, PathBuf>,
) -> anyhow::Result<Vec<SheetWriteRequest>> {
    let mut updates = BTreeMap::<String, Vec<CellUpdate>>::new();
    for operation in operations {
        match operation {
            SpreadsheetBatchOperation::WriteRows { sheet, start, rows } => {
                append_row_updates(&mut updates, sheet, *start, rows)?;
            }
            SpreadsheetBatchOperation::WriteColumns {
                sheet,
                start,
                columns,
            } => {
                append_column_updates(&mut updates, sheet, *start, columns)?;
            }
            SpreadsheetBatchOperation::CopyRows {
                source_path,
                source_sheet,
                source_start,
                row_count,
                column_count,
                destination_sheet,
                destination_start,
                content_mode,
            }
            | SpreadsheetBatchOperation::CopyColumns {
                source_path,
                source_sheet,
                source_start,
                row_count,
                column_count,
                destination_sheet,
                destination_start,
                content_mode,
            } => {
                let staged_source = copy_sources.get(source_path.trim()).with_context(|| {
                    format!("spreadsheet copy source {source_path:?} was not staged")
                })?;
                let range = counted_spreadsheet_range(
                    source_start.row,
                    source_start.column,
                    *row_count,
                    *column_count,
                )?;
                let read = crate::spreadsheet::read_range(&ReadRangeRequest {
                    path: staged_source.clone(),
                    sheet: source_sheet.clone(),
                    range,
                })?;
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
        }
    }
    Ok(updates
        .into_iter()
        .map(|(name, cells)| SheetWriteRequest {
            name,
            visibility: None,
            cells,
        })
        .collect())
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
    let value = serde_json::to_value(&result)?;
    let output = serde_json::to_string_pretty(&value)?;
    let mut content = vec![ModelContentPart::json(value.clone())];
    let mut metadata = json!({
        "toolName": "spreadsheet",
        "action": action,
        "success": true
    });
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

fn spreadsheet_error_result(
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
        SpreadsheetResult::TemplateFilled(result) => {
            if let Some(source) = source {
                result.source = source.to_path_buf();
            }
            if let Some(output) = output {
                result.output = output.to_path_buf();
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

fn ensure_xlsx_path(path: &Path) -> anyhow::Result<()> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
    {
        Ok(())
    } else {
        anyhow::bail!("spreadsheet tool supports only .xlsx files")
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

fn ensure_spreadsheet_source_path(path: &Path) -> anyhow::Result<SpreadsheetFileFormat> {
    SpreadsheetFileFormat::from_path(path).with_context(|| {
        format!(
            "unsupported spreadsheet source {}; expected one of {}",
            path.display(),
            SpreadsheetFileFormat::ATTACHMENT_EXTENSIONS.join(", ")
        )
    })
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
