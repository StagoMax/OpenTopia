use super::super::{
    spreadsheet_tool::{
        SpreadsheetColumnCopy, SpreadsheetCopyContentMode, SpreadsheetRangeConversion,
        SpreadsheetRangeCopy, SpreadsheetRowCopy,
    },
    ToolExecutionPolicy, ToolInvocationContext, TypedTool,
};
use super::common::{
    execute_action, mutation_intent, mutation_policy, parse_column_name, parse_row_conditions,
    SpreadsheetRowConditionInput,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolResult};
use crate::spreadsheet::{
    parse_a1_address, parse_a1_range, CellAddress, CellUpdate, DelimitedFormat,
    DelimitedFormulaMode, SheetWriteRequest, SpreadsheetCellInput, SpreadsheetFilterMatchMode,
    SpreadsheetValueTransform, MAX_WRITE_UPDATES,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetWriteRangeInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    sheet: String,
    /// First destination cell in Excel A1 notation.
    start: String,
    #[schemars(length(min = 1, max = 10000))]
    rows: Vec<Vec<SpreadsheetCellInput>>,
}

pub struct SpreadsheetWriteRangeTool;

#[async_trait]
impl TypedTool for SpreadsheetWriteRangeTool {
    type Input = SpreadsheetWriteRangeInput;

    fn name(&self) -> &str {
        "spreadsheet_write_range"
    }

    fn description(&self) -> &str {
        "Write a typed rectangular value matrix. Set template when the output should be rebuilt from a template; reruns replace the same output path."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        mutation_policy(
            [input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        mutation_intent(
            [input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
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
                "action": "write_rows",
                "path": input.path,
                "template": input.template,
                "sheet": input.sheet,
                "start": parse_a1_address(&input.start)?,
                "rows": input.rows
            }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetCopyRangesInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    #[schemars(length(min = 1, max = 64))]
    copies: Vec<SpreadsheetRangeCopyInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetRangeCopyInput {
    source_path: String,
    source_sheet: String,
    /// Source rectangle in Excel A1 notation.
    source_range: String,
    destination_sheet: String,
    /// First destination cell in Excel A1 notation.
    destination_start: String,
    #[serde(default)]
    content_mode: SpreadsheetCopyContentMode,
}

pub struct SpreadsheetCopyRangesTool;

#[async_trait]
impl TypedTool for SpreadsheetCopyRangesTool {
    type Input = SpreadsheetCopyRangesInput;

    fn name(&self) -> &str {
        "spreadsheet_copy_ranges"
    }

    fn description(&self) -> &str {
        "Copy one or more rectangular ranges into one spreadsheet in a single transaction. Set template to rebuild the output from that workbook; reruns replace the same output path."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let reads = input
            .copies
            .iter()
            .map(|copy| copy.source_path.clone())
            .chain(input.template.clone())
            .chain([input.path.clone()]);
        mutation_policy(reads, [input.path.clone()])
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        let reads = input
            .copies
            .iter()
            .map(|copy| copy.source_path.clone())
            .chain(input.template.clone())
            .chain([input.path.clone()]);
        mutation_intent(reads, [input.path.clone()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let copies = input
            .copies
            .into_iter()
            .map(|copy| {
                let range = parse_a1_range(&copy.source_range)?;
                Ok(SpreadsheetRangeCopy {
                    source_path: copy.source_path,
                    source_sheet: copy.source_sheet,
                    source_start: range.start,
                    row_count: u32::try_from(range.row_count().expect("validated A1 range"))
                        .map_err(|_| anyhow::anyhow!("copy range has too many rows"))?,
                    column_count: u32::try_from(range.column_count().expect("validated A1 range"))
                        .map_err(|_| anyhow::anyhow!("copy range has too many columns"))?,
                    destination_sheet: copy.destination_sheet,
                    destination_start: parse_a1_address(&copy.destination_start)?,
                    content_mode: copy.content_mode,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let cell_count = copies.iter().try_fold(0u64, |total, copy| {
            let cells = u64::from(copy.row_count) * u64::from(copy.column_count);
            total
                .checked_add(cells)
                .ok_or_else(|| anyhow::anyhow!("combined copy range is too large"))
        })?;
        anyhow::ensure!(
            cell_count <= MAX_WRITE_UPDATES as u64,
            "copy ranges contain more than {MAX_WRITE_UPDATES} cells"
        );
        execute_action(
            self.name(),
            call_id,
            json!({
                "action": "copy_ranges",
                "path": input.path,
                "template": input.template,
                "copies": copies
            }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetColumnCopyInput {
    /// Source column name, for example A or BG.
    source: String,
    /// Destination column name, for example A or K.
    destination: String,
    /// Optional typed conversions applied while copying this column.
    #[serde(default)]
    #[schemars(length(max = 8))]
    transforms: Vec<SpreadsheetValueTransform>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetCopyRowsInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    source_path: String,
    source_sheet: String,
    /// Header row of the source table, using Excel's one-based row numbering.
    /// The data range is inferred from the referenced columns and worksheet contents.
    #[schemars(range(min = 1, max = 1048576))]
    source_header_row: u32,
    /// First source data row, using Excel's one-based row numbering. This may be
    /// later than the header when a table contains description or instruction rows.
    #[schemars(range(min = 1, max = 1048576))]
    source_data_row: u32,
    destination_sheet: String,
    /// Header row of the destination table, using Excel's one-based row numbering.
    /// Data is written immediately below this row after mapped headers are validated.
    #[schemars(range(min = 1, max = 1048576))]
    destination_header_row: u32,
    #[schemars(length(min = 1, max = 256))]
    columns: Vec<SpreadsheetColumnCopyInput>,
    #[schemars(length(min = 1, max = 32))]
    conditions: Vec<SpreadsheetRowConditionInput>,
    #[serde(default)]
    match_mode: SpreadsheetFilterMatchMode,
    #[serde(default)]
    content_mode: SpreadsheetCopyContentMode,
}

pub struct SpreadsheetCopyRowsTool;

#[async_trait]
impl TypedTool for SpreadsheetCopyRowsTool {
    type Input = SpreadsheetCopyRowsInput;

    fn name(&self) -> &str {
        "spreadsheet_copy_rows"
    }

    fn description(&self) -> &str {
        "Filter source rows, map columns, and apply optional typed column conversions while copying directly into a destination workbook in one operation."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        mutation_policy(
            [input.path.clone(), input.source_path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        mutation_intent(
            [input.path.clone(), input.source_path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        anyhow::ensure!(
            input.source_header_row > 0,
            "source_header_row must be at least 1"
        );
        anyhow::ensure!(
            input.source_data_row > input.source_header_row,
            "source_data_row must be after source_header_row"
        );
        anyhow::ensure!(
            input.destination_header_row > 0,
            "destination_header_row must be at least 1"
        );
        anyhow::ensure!(!input.columns.is_empty(), "columns must not be empty");
        anyhow::ensure!(!input.conditions.is_empty(), "conditions must not be empty");
        let mut destination_columns = BTreeMap::new();
        let columns = input
            .columns
            .into_iter()
            .map(|column| {
                let source_column = parse_column_name(&column.source)?;
                let destination_column = parse_column_name(&column.destination)?;
                anyhow::ensure!(
                    destination_columns.insert(destination_column, ()).is_none(),
                    "destination columns must be unique"
                );
                Ok(SpreadsheetColumnCopy {
                    source_column,
                    destination_column,
                    transforms: column.transforms,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let conditions = parse_row_conditions(input.conditions)?;
        let column_count = u64::try_from(columns.len())?;
        let mut result = execute_action(
            self.name(),
            call_id,
            json!({
                "action": "copy_rows",
                "path": input.path,
                "template": input.template,
                "copy": SpreadsheetRowCopy {
                    source_path: input.source_path,
                    source_sheet: input.source_sheet,
                    source_header_row: input.source_header_row - 1,
                    source_data_row: input.source_data_row - 1,
                    destination_sheet: input.destination_sheet,
                    destination_header_row: input.destination_header_row - 1,
                    columns,
                    conditions,
                    match_mode: input.match_mode,
                    content_mode: input.content_mode,
                }
            }),
            ctx,
        )
        .await?;
        if result.metadata["success"] == true {
            let applied_updates = result.content.iter().find_map(|part| match part {
                ModelContentPart::Json { value } => value
                    .pointer("/result/appliedUpdates")
                    .and_then(serde_json::Value::as_u64),
                _ => None,
            });
            if let (Some(applied_updates), Some(metadata)) =
                (applied_updates, result.metadata.as_object_mut())
            {
                metadata.insert(
                    "copiedRows".to_string(),
                    json!(applied_updates / column_count),
                );
            }
        }
        Ok(result)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetRangeFill {
    sheet: String,
    /// Destination range in Excel A1 notation.
    range: String,
    value: SpreadsheetCellInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetFillRangesInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    #[schemars(length(min = 1, max = 64))]
    fills: Vec<SpreadsheetRangeFill>,
}

pub struct SpreadsheetFillRangesTool;

#[async_trait]
impl TypedTool for SpreadsheetFillRangesTool {
    type Input = SpreadsheetFillRangesInput;

    fn name(&self) -> &str {
        "spreadsheet_fill_ranges"
    }

    fn description(&self) -> &str {
        "Fill one or more bounded spreadsheet ranges with typed constants in one transaction. Set template to rebuild an output from that workbook."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        mutation_policy(
            [input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        mutation_intent(
            [input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let sheets = fill_requests(input.fills)?;
        execute_action(
            self.name(),
            call_id,
            json!({
                "action": "write",
                "path": input.path,
                "template": input.template,
                "sheets": sheets
            }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetConvertRangesInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    #[schemars(length(min = 1, max = 64))]
    conversions: Vec<SpreadsheetRangeConversionInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetRangeConversionInput {
    sheet: String,
    /// Conversion range in Excel A1 notation.
    range: String,
    #[schemars(length(min = 1, max = 8))]
    transforms: Vec<SpreadsheetValueTransform>,
}

pub struct SpreadsheetConvertRangesTool;

#[async_trait]
impl TypedTool for SpreadsheetConvertRangesTool {
    type Input = SpreadsheetConvertRangesInput;

    fn name(&self) -> &str {
        "spreadsheet_convert_ranges"
    }

    fn description(&self) -> &str {
        "Convert populated values in one or more ranges in one transaction. Set template to rebuild an output from that workbook. Date parsing requires input_format and output_number_format."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        mutation_policy(
            [input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        mutation_intent(
            [input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let conversions = input
            .conversions
            .into_iter()
            .map(|conversion| {
                Ok(SpreadsheetRangeConversion {
                    sheet: conversion.sheet,
                    range: parse_a1_range(&conversion.range)?,
                    transforms: conversion.transforms,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let cell_count = conversions.iter().try_fold(0u64, |total, conversion| {
            let cells = conversion
                .range
                .cell_count()
                .ok_or_else(|| anyhow::anyhow!("range end must not precede range start"))?;
            total
                .checked_add(cells)
                .ok_or_else(|| anyhow::anyhow!("combined conversion range is too large"))
        })?;
        anyhow::ensure!(
            cell_count <= MAX_WRITE_UPDATES as u64,
            "conversion ranges contain more than {MAX_WRITE_UPDATES} cells"
        );
        execute_action(
            self.name(),
            call_id,
            json!({
                "action": "convert_ranges",
                "path": input.path,
                "template": input.template,
                "conversions": conversions
            }),
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetExportDelimitedInput {
    path: String,
    output_path: String,
    sheet: String,
    #[serde(default)]
    /// Optional export range in Excel A1 notation.
    range: Option<String>,
    #[serde(default)]
    format: Option<DelimitedFormat>,
    #[serde(default)]
    formula_mode: DelimitedFormulaMode,
}

pub struct SpreadsheetExportDelimitedTool;

#[async_trait]
impl TypedTool for SpreadsheetExportDelimitedTool {
    type Input = SpreadsheetExportDelimitedInput;

    fn name(&self) -> &str {
        "spreadsheet_export_delimited"
    }

    fn description(&self) -> &str {
        "Export one spreadsheet sheet or range to a CSV or TSV file path."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        mutation_policy([input.path.clone()], [input.output_path.clone()])
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        mutation_intent([input.path.clone()], [input.output_path.clone()])
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
                "action": "export_delimited",
                "path": input.path,
                "outputPath": input.output_path,
                "sheet": input.sheet,
                "range": input.range.as_deref().map(parse_a1_range).transpose()?,
                "format": input.format,
                "formulaMode": input.formula_mode
            }),
            ctx,
        )
        .await
    }
}

fn fill_requests(fills: Vec<SpreadsheetRangeFill>) -> anyhow::Result<Vec<SheetWriteRequest>> {
    let mut updates = BTreeMap::<String, BTreeMap<CellAddress, CellUpdate>>::new();
    let mut cell_count = 0u64;
    for fill in fills {
        let range = parse_a1_range(&fill.range)?;
        let cells = range
            .cell_count()
            .ok_or_else(|| anyhow::anyhow!("range end must not precede range start"))?;
        cell_count = cell_count
            .checked_add(cells)
            .ok_or_else(|| anyhow::anyhow!("combined fill range is too large"))?;
        anyhow::ensure!(
            cell_count <= MAX_WRITE_UPDATES as u64,
            "fill ranges contain more than {MAX_WRITE_UPDATES} cells"
        );
        let sheet = updates.entry(fill.sheet).or_default();
        for row in range.start.row..=range.end.row {
            for column in range.start.column..=range.end.column {
                let address = CellAddress { row, column };
                anyhow::ensure!(
                    sheet
                        .insert(
                            address,
                            CellUpdate {
                                address,
                                value: fill.value.clone(),
                                style_from: Some(CellAddress {
                                    row: range.start.row,
                                    column,
                                }),
                            },
                        )
                        .is_none(),
                    "fill ranges must not overlap"
                );
            }
        }
    }
    Ok(updates
        .into_iter()
        .map(|(name, cells)| SheetWriteRequest {
            name,
            visibility: None,
            cells: cells.into_iter().map(|(_, update)| update).collect(),
        })
        .collect())
}
