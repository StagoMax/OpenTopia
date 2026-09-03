use super::backend::{execute_backend, validate_arguments};
use super::contracts::*;
use super::{DocumentOperationHandler, OperationEffect};
use crate::model::{ModelContentPart, ToolResult};
use crate::spreadsheet::{
    transform_cell_input, SpreadsheetCell, SpreadsheetCellInput, SpreadsheetCellValue,
    SpreadsheetResult,
};
use crate::tools::document_session::{
    get_selection, insert_selection, DocumentSession, TabularSelection,
};
use crate::tools::ToolInvocationContext;
use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

const SELECTION_PREVIEW_ROWS: usize = 5;

struct FilterRowsHandler;

#[async_trait]
impl DocumentOperationHandler for FilterRowsHandler {
    fn name(&self) -> &'static str {
        "filter_rows"
    }

    fn description(&self) -> &'static str {
        "Filter up to 2,000 rows and retain the complete result as a server-side selection."
    }

    fn effect(&self) -> OperationEffect {
        OperationEffect::SessionMutation
    }

    fn requires_editable_document(&self) -> bool {
        false
    }

    fn arguments_schema(&self) -> Value {
        schema::<FilterRowsArguments>()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name(), &self.arguments_schema(), &arguments)?;
        let parsed: FilterRowsArguments = parse(arguments.clone())?;
        let Value::Object(mut input) = arguments else {
            anyhow::bail!("document_execute.filter_rows.arguments must be an object")
        };
        input
            .entry("maxResults".to_string())
            .or_insert_with(|| json!(2000));
        let (binding, value) = session.resource.read_binding()?;
        input.insert(binding.to_string(), value);
        input.insert("action".to_string(), json!("filter_rows"));
        let backend = execute_backend(call_id, self.name(), input, ctx.clone()).await?;
        if backend.metadata["success"] != true {
            return Ok(backend);
        }
        let spreadsheet_result = backend
            .content
            .iter()
            .find_map(|part| match part {
                ModelContentPart::Json { value } => {
                    serde_json::from_value::<SpreadsheetResult>(value.clone()).ok()
                }
                _ => None,
            })
            .ok_or_else(|| anyhow::anyhow!("filter_rows backend returned no structured result"))?;
        let SpreadsheetResult::RowsFiltered(filtered) = spreadsheet_result else {
            anyhow::bail!("filter_rows backend returned the wrong result type")
        };
        let rows = filtered
            .rows
            .iter()
            .map(|row| row.iter().map(cell_to_input).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let source_columns = (parsed.range.start.column..=parsed.range.end.column)
            .map(Some)
            .collect::<Vec<_>>();
        let selection = insert_selection(
            ctx.thread_id,
            session.id,
            rows,
            filtered.matched_row_indices,
            source_columns,
        )?;
        selection_result(
            call_id,
            self.name(),
            &selection,
            json!({
                "scannedRows": filtered.scanned_rows,
                "truncated": filtered.truncated,
                "sheet": parsed.sheet,
                "range": parsed.range,
            }),
        )
    }
}

struct SelectColumnsHandler;

#[async_trait]
impl DocumentOperationHandler for SelectColumnsHandler {
    fn name(&self) -> &'static str {
        "select_columns"
    }

    fn description(&self) -> &'static str {
        "Project and reorder columns in a server-side selection; duplicate source positions are allowed."
    }

    fn effect(&self) -> OperationEffect {
        OperationEffect::SessionMutation
    }

    fn requires_editable_document(&self) -> bool {
        false
    }

    fn arguments_schema(&self) -> Value {
        schema::<SelectColumnsArguments>()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name(), &self.arguments_schema(), &arguments)?;
        let parsed: SelectColumnsArguments = parse(arguments)?;
        let selection = source_selection(parsed.selection_id, session, &ctx)?;
        let positions = parsed
            .columns
            .iter()
            .map(|column| usize::try_from(*column).unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        for position in &positions {
            anyhow::ensure!(
                *position < selection.width(),
                "selection column {position} does not exist; width is {}",
                selection.width()
            );
        }
        let rows = selection
            .rows
            .iter()
            .map(|row| {
                positions
                    .iter()
                    .map(|position| row[*position].clone())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let source_columns = positions
            .iter()
            .map(|position| selection.source_columns[*position])
            .collect::<Vec<_>>();
        let derived = insert_selection(
            ctx.thread_id,
            selection.source_document_id,
            rows,
            selection.source_rows,
            source_columns,
        )?;
        selection_result(call_id, self.name(), &derived, json!({}))
    }
}

struct SetConstantColumnHandler;

#[async_trait]
impl DocumentOperationHandler for SetConstantColumnHandler {
    fn name(&self) -> &'static str {
        "set_constant_column"
    }

    fn description(&self) -> &'static str {
        "Insert or replace one column with the same typed value for every selected row."
    }

    fn effect(&self) -> OperationEffect {
        OperationEffect::SessionMutation
    }

    fn requires_editable_document(&self) -> bool {
        false
    }

    fn arguments_schema(&self) -> Value {
        schema::<SetConstantColumnArguments>()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name(), &self.arguments_schema(), &arguments)?;
        let parsed: SetConstantColumnArguments = parse(arguments)?;
        let selection = source_selection(parsed.selection_id, session, &ctx)?;
        let column = usize::try_from(parsed.column).unwrap_or(usize::MAX);
        let mut rows = selection.rows;
        let mut source_columns = selection.source_columns;
        match parsed.mode {
            SetConstantMode::Replace => {
                anyhow::ensure!(
                    column < source_columns.len(),
                    "selection column {column} does not exist; width is {}",
                    source_columns.len()
                );
                for row in &mut rows {
                    row[column] = parsed.value.clone();
                }
                source_columns[column] = None;
            }
            SetConstantMode::Insert => {
                anyhow::ensure!(
                    column <= source_columns.len(),
                    "selection insert column {column} exceeds width {}",
                    source_columns.len()
                );
                for row in &mut rows {
                    row.insert(column, parsed.value.clone());
                }
                source_columns.insert(column, None);
            }
        }
        let derived = insert_selection(
            ctx.thread_id,
            selection.source_document_id,
            rows,
            selection.source_rows,
            source_columns,
        )?;
        selection_result(call_id, self.name(), &derived, json!({}))
    }
}

struct ConvertColumnHandler;

#[async_trait]
impl DocumentOperationHandler for ConvertColumnHandler {
    fn name(&self) -> &'static str {
        "convert_column"
    }

    fn description(&self) -> &'static str {
        "Apply one typed conversion to one column in a server-side selection."
    }

    fn effect(&self) -> OperationEffect {
        OperationEffect::SessionMutation
    }

    fn requires_editable_document(&self) -> bool {
        false
    }

    fn arguments_schema(&self) -> Value {
        schema::<ConvertColumnArguments>()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name(), &self.arguments_schema(), &arguments)?;
        let parsed: ConvertColumnArguments = parse(arguments)?;
        let selection = source_selection(parsed.selection_id, session, &ctx)?;
        let column = usize::try_from(parsed.column).unwrap_or(usize::MAX);
        anyhow::ensure!(
            column < selection.width(),
            "selection column {column} does not exist; width is {}",
            selection.width()
        );
        let physical_column = selection.source_columns[column].unwrap_or(parsed.column);
        let mut rows = selection.rows;
        for (offset, row) in rows.iter_mut().enumerate() {
            let source_row = selection.source_rows[offset];
            row[column] = transform_cell_input(
                row[column].clone(),
                std::slice::from_ref(&parsed.transform),
                source_row,
                physical_column,
            )?;
        }
        let derived = insert_selection(
            ctx.thread_id,
            selection.source_document_id,
            rows,
            selection.source_rows,
            selection.source_columns,
        )?;
        selection_result(call_id, self.name(), &derived, json!({}))
    }
}

struct WriteSelectionHandler;

#[async_trait]
impl DocumentOperationHandler for WriteSelectionHandler {
    fn name(&self) -> &'static str {
        "write_selection"
    }

    fn description(&self) -> &'static str {
        "Write a complete server-side selection into this spreadsheet without returning its rows to the model."
    }

    fn effect(&self) -> OperationEffect {
        OperationEffect::FileMutation
    }

    fn requires_editable_document(&self) -> bool {
        true
    }

    fn arguments_schema(&self) -> Value {
        schema::<WriteSelectionArguments>()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name(), &self.arguments_schema(), &arguments)?;
        let parsed: WriteSelectionArguments = parse(arguments)?;
        let selection = get_selection(parsed.selection_id, ctx.thread_id)?;
        let path = session
            .resource
            .file_path()
            .ok_or_else(|| anyhow::anyhow!("write_selection requires an editable file document"))?;
        let row_count = selection.rows.len();
        let column_count = selection.width();
        let input = json!({
            "action": "write_rows",
            "path": path,
            "sheet": parsed.sheet,
            "start": parsed.start,
            "rows": selection.rows,
        });
        let Value::Object(input) = input else {
            unreachable!()
        };
        let mut result = execute_backend(call_id, self.name(), input, ctx).await?;
        if let Some(metadata) = result.metadata.as_object_mut() {
            metadata.insert("selectionId".to_string(), json!(parsed.selection_id));
            if metadata.get("success") == Some(&Value::Bool(true)) {
                metadata.insert("rowsWritten".to_string(), json!(row_count));
                metadata.insert("columnsWritten".to_string(), json!(column_count));
            }
        }
        Ok(result)
    }
}

fn source_selection(
    selection_id: Uuid,
    session: &DocumentSession,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<TabularSelection> {
    let selection = get_selection(selection_id, ctx.thread_id)?;
    anyhow::ensure!(
        selection.source_document_id == session.id,
        "selection {selection_id} belongs to document {}, not {}",
        selection.source_document_id,
        session.id
    );
    Ok(selection)
}

fn cell_to_input(cell: &SpreadsheetCell) -> SpreadsheetCellInput {
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

fn selection_result(
    call_id: Uuid,
    operation: &str,
    selection: &TabularSelection,
    extra: Value,
) -> anyhow::Result<ToolResult> {
    let mut payload = json!({
        "selectionId": selection.id,
        "sourceDocumentId": selection.source_document_id,
        "rowCount": selection.rows.len(),
        "columnCount": selection.width(),
        "previewRows": selection.rows.iter().take(SELECTION_PREVIEW_ROWS).collect::<Vec<_>>(),
        "previewTruncated": selection.rows.len() > SELECTION_PREVIEW_ROWS,
    });
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        payload.extend(extra.clone());
    }
    Ok(ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&payload)?,
        content: vec![ModelContentPart::json(payload)],
        metadata: json!({
            "toolName": "document_execute",
            "operation": operation,
            "success": true,
            "selectionId": selection.id,
        }),
    })
}

pub(super) fn handlers() -> Vec<Box<dyn DocumentOperationHandler>> {
    vec![
        Box::new(FilterRowsHandler),
        Box::new(SelectColumnsHandler),
        Box::new(SetConstantColumnHandler),
        Box::new(ConvertColumnHandler),
        Box::new(WriteSelectionHandler),
    ]
}
