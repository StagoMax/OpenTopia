use super::super::{ToolExecutionPolicy, ToolInvocationContext, TypedTool};
use super::common::{
    execute_action, observation_intent, parse_row_conditions, read_policy,
    SpreadsheetRowConditionInput,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolResult};
use crate::spreadsheet::{
    format_a1_address, format_a1_range, parse_a1_range, CellAddress, CellRange, SheetRangeRequest,
    SpreadsheetExpectedCellType, SpreadsheetFilterMatchMode, SpreadsheetFilterReturnMode,
    SpreadsheetRangeValidation, SpreadsheetSheetValidation, SpreadsheetTextMatchMode,
    MAX_READ_CELLS, MAX_READ_COLUMNS, MAX_READ_RANGES, MAX_READ_ROWS,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetPathInput {
    /// Real workbook path from the attachment manifest or filesystem.
    path: String,
}

pub struct SpreadsheetInspectTool;

#[async_trait]
impl TypedTool for SpreadsheetInspectTool {
    type Input = SpreadsheetPathInput;

    fn name(&self) -> &str {
        "spreadsheet_inspect"
    }

    fn description(&self) -> &str {
        "Inspect a spreadsheet at a real file path and return sheets, used ranges, populated-cell counts, data-validation guidance, and cell comments."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        read_policy([input.path.clone()])
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        observation_intent([input.path.clone()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        execute_action(
            self.name(),
            call_id,
            json!({ "action": "inspect", "path": input.path }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetReadItemInput {
    path: String,
    sheet: String,
    /// Excel A1 range, for example A1:K20.
    range: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetReadRangesInput {
    #[schemars(length(min = 1, max = 64))]
    reads: Vec<SpreadsheetReadItemInput>,
}

pub struct SpreadsheetReadRangesTool;

#[async_trait]
impl TypedTool for SpreadsheetReadRangesTool {
    type Input = SpreadsheetReadRangesInput;

    fn name(&self) -> &str {
        "spreadsheet_read_ranges"
    }

    fn description(&self) -> &str {
        "Read several bounded A1 ranges in one call. Workbooks are loaded once per path and results preserve request order."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        read_policy(input.reads.iter().map(|read| read.path.clone()))
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        observation_intent(input.reads.iter().map(|read| read.path.clone()))
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        anyhow::ensure!(!input.reads.is_empty(), "reads must not be empty");
        anyhow::ensure!(
            input.reads.len() <= MAX_READ_RANGES,
            "reads are limited to {MAX_READ_RANGES} ranges"
        );
        let page_budget = (MAX_READ_CELLS / input.reads.len() as u64).max(1);
        let mut total_cells = 0_u64;
        let mut requested_cells = 0_u64;
        let mut pages = vec![Value::Null; input.reads.len()];
        let mut groups = BTreeMap::<String, Vec<(usize, SheetRangeRequest)>>::new();
        for (index, read) in input.reads.iter().enumerate() {
            let requested = parse_a1_range(&read.range)?;
            requested_cells = requested_cells
                .checked_add(requested.cell_count().expect("validated A1 range"))
                .ok_or_else(|| anyhow::anyhow!("combined requested range is too large"))?;
            let requested_columns = requested.column_count().expect("validated A1 range");
            let requested_rows = requested.row_count().expect("validated A1 range");
            let returned_columns = requested_columns
                .min(MAX_READ_COLUMNS)
                .min(page_budget)
                .max(1);
            let returned_rows = requested_rows
                .min(MAX_READ_ROWS)
                .min((page_budget / returned_columns).max(1));
            let range = CellRange {
                start: requested.start,
                end: CellAddress {
                    row: requested.start.row + returned_rows as u32 - 1,
                    column: requested.start.column + returned_columns as u32 - 1,
                },
            };
            total_cells = total_cells
                .checked_add(range.cell_count().expect("validated A1 range"))
                .ok_or_else(|| anyhow::anyhow!("combined read range is too large"))?;
            let has_more_rows = returned_rows < requested_rows;
            let has_more_columns = returned_columns < requested_columns;
            pages[index] = json!({
                "requestedRange": format_a1_range(requested),
                "returnedRange": format_a1_range(range),
                "hasMore": has_more_rows || has_more_columns,
                "nextStartRow": has_more_rows.then(|| format_a1_address(CellAddress {
                    row: range.end.row + 1,
                    column: requested.start.column,
                })),
                "nextStartColumn": has_more_columns.then(|| format_a1_address(CellAddress {
                    row: requested.start.row,
                    column: range.end.column + 1,
                }))
            });
            groups.entry(read.path.clone()).or_default().push((
                index,
                SheetRangeRequest {
                    sheet: read.sheet.clone(),
                    range,
                },
            ));
        }
        let mut ordered = vec![Value::Null; input.reads.len()];
        for (path, group) in groups {
            let result = execute_action(
                self.name(),
                call_id,
                json!({
                    "action": "read_ranges",
                    "path": path,
                    "ranges": group.iter().map(|(_, range)| range).collect::<Vec<_>>()
                }),
                ctx.clone(),
            )
            .await?;
            if result.metadata["success"] != true {
                return Ok(result);
            }
            let ranges = result
                .content
                .iter()
                .find_map(|part| match part {
                    ModelContentPart::Json { value } => {
                        value.pointer("/result/ranges").and_then(Value::as_array)
                    }
                    _ => None,
                })
                .ok_or_else(|| anyhow::anyhow!("spreadsheet backend omitted range results"))?;
            anyhow::ensure!(
                ranges.len() == group.len(),
                "spreadsheet backend returned an unexpected range count"
            );
            for ((index, _), range) in group.into_iter().zip(ranges.iter()) {
                ordered[index] = compact_read_range(range, pages[index].clone())?;
            }
        }

        let has_more = pages.iter().any(|page| page["hasMore"] == true);
        let value = json!({
            "reads": ordered,
            "requestedCells": requested_cells,
            "returnedCells": total_cells,
            "hasMore": has_more
        });
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({
                "toolName": self.name(),
                "success": true
            }),
        })
    }
}

fn compact_read_range(range: &Value, page: Value) -> anyhow::Result<Value> {
    let object = range
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("spreadsheet backend returned an invalid range result"))?;
    let rows = object
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("spreadsheet backend omitted range rows"))?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| anyhow::anyhow!("spreadsheet backend returned an invalid row"))?
                .iter()
                .map(compact_read_cell)
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(json!({
        "path": object.get("path").cloned().unwrap_or(Value::Null),
        "sheet": object.get("sheet").cloned().unwrap_or(Value::Null),
        "range": page.get("returnedRange").cloned().unwrap_or(Value::Null),
        "rows": rows,
        "page": page,
    }))
}

fn compact_read_cell(cell: &Value) -> anyhow::Result<Value> {
    let object = cell
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("spreadsheet backend returned an invalid cell"))?;
    let mut value = compact_read_cell_value(
        object
            .get("value")
            .ok_or_else(|| anyhow::anyhow!("spreadsheet backend omitted a cell value"))?,
    )?;
    let Some(formula) = object.get("formula").and_then(Value::as_str) else {
        if let Some(formatted) = object.get("formatted").and_then(Value::as_str) {
            if let Some(compact) = value.as_object_mut() {
                compact.insert("formatted".to_string(), json!(formatted));
            }
        }
        return Ok(value);
    };
    let mut formula_cell = serde_json::Map::new();
    formula_cell.insert("formula".to_string(), json!(formula));
    formula_cell.insert("value".to_string(), value);
    if let Some(formatted) = object.get("formatted").and_then(Value::as_str) {
        formula_cell.insert("formatted".to_string(), json!(formatted));
    }
    Ok(Value::Object(formula_cell))
}

fn compact_read_cell_value(value: &Value) -> anyhow::Result<Value> {
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!("spreadsheet backend returned an invalid typed cell value")
    })?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("spreadsheet backend omitted the cell value type"))?;
    let content = object.get("value").cloned().unwrap_or(Value::Null);
    Ok(match kind {
        "empty" => Value::Null,
        "string" | "integer" | "number" | "boolean" => content,
        "date_time" | "date_time_iso" => json!({ "date_time": content }),
        "duration_iso" => json!({ "duration": content }),
        "error" => json!({ "error": content }),
        other => anyhow::bail!("spreadsheet backend returned unknown cell value type {other:?}"),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetFindInput {
    path: String,
    #[serde(default)]
    sheet: Option<String>,
    #[serde(default)]
    /// Optional search range in Excel A1 notation.
    range: Option<String>,
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
}

pub struct SpreadsheetFindTool;

#[async_trait]
impl TypedTool for SpreadsheetFindTool {
    type Input = SpreadsheetFindInput;

    fn name(&self) -> &str {
        "spreadsheet_find"
    }

    fn description(&self) -> &str {
        "Find matching values or formulas in a spreadsheet without loading the workbook into model context."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        read_policy([input.path.clone()])
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        observation_intent([input.path.clone()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        execute_action(
            self.name(),
            call_id,
            json!({
                "action": "find",
                "path": input.path,
                "sheet": input.sheet,
                "range": input.range.as_deref().map(parse_a1_range).transpose()?,
                "query": input.query,
                "matchMode": input.match_mode,
                "caseSensitive": input.case_sensitive,
                "includeFormulas": input.include_formulas,
                "maxResults": input.max_results
            }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetFilterRowsInput {
    path: String,
    sheet: String,
    /// Rows and columns to scan in Excel A1 notation.
    range: String,
    #[schemars(length(min = 1, max = 32))]
    conditions: Vec<SpreadsheetRowConditionInput>,
    #[serde(default)]
    match_mode: Option<SpreadsheetFilterMatchMode>,
    /// `summary` returns only the exact count. Use `indices` or `rows` only when those
    /// values are needed by the caller.
    #[serde(default)]
    return_mode: SpreadsheetFilterReturnMode,
    #[serde(default)]
    #[schemars(range(min = 1, max = 2000))]
    max_results: Option<usize>,
}

pub struct SpreadsheetFilterRowsTool;

#[async_trait]
impl TypedTool for SpreadsheetFilterRowsTool {
    type Input = SpreadsheetFilterRowsInput;

    fn name(&self) -> &str {
        "spreadsheet_filter_rows"
    }

    fn description(&self) -> &str {
        "Query rows matching typed conditions. By default it returns only an exact count; request indices or rows explicitly. File-to-file editing tools do not route rows through the model."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        read_policy([input.path.clone()])
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        observation_intent([input.path.clone()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        execute_action(
            self.name(),
            call_id,
            json!({
                "action": "filter_rows",
                "path": input.path,
                "sheet": input.sheet,
                "range": parse_a1_range(&input.range)?,
                "conditions": parse_row_conditions(input.conditions)?,
                "filterMatchMode": input.match_mode,
                "returnMode": input.return_mode,
                "maxResults": input.max_results.unwrap_or(2000)
            }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetValidateInput {
    path: String,
    #[serde(default)]
    expected_sheets: Vec<String>,
    #[serde(default)]
    expected_populated_cells: Option<u64>,
    #[serde(default)]
    sheets: Vec<SpreadsheetSheetValidationInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetSheetValidationInput {
    sheet: String,
    #[serde(default)]
    expected_rows: Option<u32>,
    #[serde(default)]
    expected_data_rows: Option<u32>,
    /// Optional header definition. Row numbers use Excel's one-based numbering.
    /// Omit this object for headerless sheets.
    #[serde(default)]
    header: Option<SpreadsheetHeaderValidationInput>,
    #[serde(default)]
    ranges: Vec<SpreadsheetRangeValidationInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetHeaderValidationInput {
    #[schemars(range(min = 1, max = 1048576))]
    row: u32,
    #[serde(default)]
    required: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetRangeValidationInput {
    /// Range to validate in Excel A1 notation.
    range: String,
    #[serde(default)]
    expected_type: Option<SpreadsheetExpectedCellType>,
    #[serde(default)]
    expected_number_format: Option<String>,
    #[serde(default)]
    allow_blank: bool,
}

pub struct SpreadsheetValidateTool;

#[async_trait]
impl TypedTool for SpreadsheetValidateTool {
    type Input = SpreadsheetValidateInput;

    fn name(&self) -> &str {
        "spreadsheet_validate"
    }

    fn description(&self) -> &str {
        "Reopen a spreadsheet and validate its structure, optional headers, row counts, populated cells, value types, and number formats."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        read_policy([input.path.clone()])
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        observation_intent([input.path.clone()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let sheets = input
            .sheets
            .into_iter()
            .map(|sheet| {
                let (header_row, required_headers) = match sheet.header {
                    Some(header) => {
                        anyhow::ensure!(header.row > 0, "header.row must be at least 1");
                        (Some(header.row - 1), header.required)
                    }
                    None => (None, Vec::new()),
                };
                Ok(SpreadsheetSheetValidation {
                    sheet: sheet.sheet,
                    expected_rows: sheet.expected_rows,
                    expected_data_rows: sheet.expected_data_rows,
                    header_row,
                    required_headers,
                    ranges: sheet
                        .ranges
                        .into_iter()
                        .map(|range| {
                            Ok(SpreadsheetRangeValidation {
                                range: parse_a1_range(&range.range)?,
                                expected_type: range.expected_type,
                                expected_number_format: range.expected_number_format,
                                allow_blank: range.allow_blank,
                            })
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        execute_action(
            self.name(),
            call_id,
            json!({
                "action": "validate",
                "path": input.path,
                "expectedSheets": input.expected_sheets,
                "expectedPopulatedCells": input.expected_populated_cells,
                "sheets": sheets
            }),
            ctx,
        )
        .await
    }
}
