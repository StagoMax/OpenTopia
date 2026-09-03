#![allow(dead_code)]

use super::super::spreadsheet_tool::SpreadsheetCopyContentMode;
use crate::spreadsheet::{
    CellAddress, CellRange, DelimitedFormat, DelimitedFormulaMode, SheetRangeRequest,
    SheetWriteRequest, SpreadsheetCellInput, SpreadsheetFilterCondition,
    SpreadsheetFilterMatchMode, SpreadsheetSheetValidation, SpreadsheetTextMatchMode,
    SpreadsheetValueTransform,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct EmptyArguments {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReadRangeArguments {
    pub(super) sheet: String,
    pub(super) range: CellRange,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReadRangesArguments {
    #[schemars(length(min = 1, max = 64))]
    pub(super) ranges: Vec<SheetRangeRequest>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReadRowsArguments {
    pub(super) sheet: String,
    pub(super) start_row: u32,
    pub(super) start_column: u32,
    #[schemars(range(min = 1))]
    pub(super) row_count: u32,
    #[schemars(range(min = 1))]
    pub(super) column_count: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FindArguments {
    #[serde(default)]
    pub(super) sheet: Option<String>,
    #[serde(default)]
    pub(super) range: Option<CellRange>,
    pub(super) query: String,
    #[serde(default)]
    pub(super) match_mode: Option<SpreadsheetTextMatchMode>,
    #[serde(default)]
    pub(super) case_sensitive: bool,
    #[serde(default)]
    pub(super) include_formulas: bool,
    #[serde(default)]
    #[schemars(range(min = 1, max = 1000))]
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FilterRowsArguments {
    pub(super) sheet: String,
    pub(super) range: CellRange,
    #[schemars(length(min = 1, max = 32))]
    pub(super) conditions: Vec<SpreadsheetFilterCondition>,
    #[serde(default)]
    pub(super) filter_match_mode: Option<SpreadsheetFilterMatchMode>,
    #[serde(default)]
    #[schemars(range(min = 1, max = 2000))]
    pub(super) max_results: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ValidateArguments {
    #[serde(default)]
    pub(super) expected_sheets: Vec<String>,
    #[serde(default)]
    pub(super) expected_populated_cells: Option<u64>,
    #[serde(default)]
    pub(super) sheets: Vec<SpreadsheetSheetValidation>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ExportDelimitedArguments {
    pub(super) output_path: String,
    pub(super) sheet: String,
    #[serde(default)]
    pub(super) range: Option<CellRange>,
    #[serde(default)]
    pub(super) format: Option<DelimitedFormat>,
    #[serde(default)]
    pub(super) formula_mode: DelimitedFormulaMode,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WriteArguments {
    #[schemars(length(max = 256))]
    pub(super) sheets: Vec<SheetWriteRequest>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WriteRowsArguments {
    pub(super) sheet: String,
    pub(super) start: CellAddress,
    #[schemars(length(min = 1, max = 10000))]
    pub(super) rows: Vec<Vec<SpreadsheetCellInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WriteColumnsArguments {
    pub(super) sheet: String,
    pub(super) start: CellAddress,
    #[schemars(length(min = 1, max = 256))]
    pub(super) columns: Vec<Vec<SpreadsheetCellInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CopyArguments {
    pub(super) source_document_id: Uuid,
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SelectColumnsArguments {
    pub(super) selection_id: Uuid,
    /// Zero-based positions in the current selection. Duplicates are allowed.
    #[schemars(length(min = 1, max = 256))]
    pub(super) columns: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum SetConstantMode {
    #[default]
    Replace,
    Insert,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SetConstantColumnArguments {
    pub(super) selection_id: Uuid,
    /// Zero-based position in the current selection. Insert permits width as an append position.
    pub(super) column: u32,
    pub(super) value: SpreadsheetCellInput,
    #[serde(default)]
    pub(super) mode: SetConstantMode,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConvertColumnArguments {
    pub(super) selection_id: Uuid,
    /// Zero-based position in the current selection.
    pub(super) column: u32,
    pub(super) transform: SpreadsheetValueTransform,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WriteSelectionArguments {
    pub(super) selection_id: Uuid,
    pub(super) sheet: String,
    pub(super) start: CellAddress,
}

pub(super) fn schema<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("document operation schema")
}

pub(super) fn parse<T: DeserializeOwned>(arguments: Value) -> anyhow::Result<T> {
    serde_json::from_value(arguments).map_err(Into::into)
}
