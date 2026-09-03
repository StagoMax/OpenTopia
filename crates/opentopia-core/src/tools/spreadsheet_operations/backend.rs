use super::contracts::*;
use super::{DocumentOperationHandler, OperationEffect};
use crate::model::ToolResult;
use crate::provider::tool_input_schema_error;
use crate::tools::document_session::{get_document, DocumentSession};
use crate::tools::spreadsheet_tool::execute_spreadsheet_backend;
use crate::tools::ToolInvocationContext;
use async_trait::async_trait;
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
enum ResourceBinding {
    Read,
    Editable,
}

struct BackendHandler {
    name: &'static str,
    action: &'static str,
    description: &'static str,
    effect: OperationEffect,
    binding: ResourceBinding,
    schema: fn() -> Value,
}

impl BackendHandler {
    const fn new(
        name: &'static str,
        description: &'static str,
        effect: OperationEffect,
        binding: ResourceBinding,
        schema: fn() -> Value,
    ) -> Self {
        Self {
            name,
            action: name,
            description,
            effect,
            binding,
            schema,
        }
    }
}

#[async_trait]
impl DocumentOperationHandler for BackendHandler {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        self.description
    }

    fn effect(&self) -> OperationEffect {
        self.effect
    }

    fn requires_editable_document(&self) -> bool {
        matches!(self.binding, ResourceBinding::Editable)
    }

    fn arguments_schema(&self) -> Value {
        (self.schema)()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name, &(self.schema)(), &arguments)?;
        let Value::Object(mut input) = arguments else {
            anyhow::bail!("{}.arguments must be an object", self.name);
        };
        bind_resource(session, self.binding, &mut input)?;
        input.insert("action".to_string(), Value::String(self.action.to_string()));
        execute_backend(call_id, self.name, input, ctx).await
    }
}

struct CopyHandler {
    name: &'static str,
}

#[async_trait]
impl DocumentOperationHandler for CopyHandler {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        "Copy a rectangular block from another open spreadsheet into this spreadsheet."
    }

    fn effect(&self) -> OperationEffect {
        OperationEffect::FileMutation
    }

    fn requires_editable_document(&self) -> bool {
        true
    }

    fn arguments_schema(&self) -> Value {
        schema::<CopyArguments>()
    }

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_arguments(self.name, &self.arguments_schema(), &arguments)?;
        let parsed: CopyArguments = parse(arguments)?;
        let source = get_document(parsed.source_document_id, ctx.thread_id)?;
        let source_path = source.resource.file_path().ok_or_else(|| {
            anyhow::anyhow!(
                "{} requires a file-backed source document; use filter_rows and write_selection for attachment data",
                self.name
            )
        })?;
        let destination_path = session
            .resource
            .file_path()
            .ok_or_else(|| anyhow::anyhow!("{} requires an editable file document", self.name))?;
        let input = serde_json::json!({
            "action": self.name,
            "path": destination_path,
            "from": source_path,
            "sourceSheet": parsed.source_sheet,
            "sourceStart": parsed.source_start,
            "rowCount": parsed.row_count,
            "columnCount": parsed.column_count,
            "destinationSheet": parsed.destination_sheet,
            "destinationStart": parsed.destination_start,
            "contentMode": parsed.content_mode,
        });
        let Value::Object(input) = input else {
            unreachable!()
        };
        execute_backend(call_id, self.name, input, ctx).await
    }
}

fn bind_resource(
    session: &DocumentSession,
    binding: ResourceBinding,
    input: &mut Map<String, Value>,
) -> anyhow::Result<()> {
    match binding {
        ResourceBinding::Read => {
            let (key, value) = session.resource.read_binding()?;
            input.insert(key.to_string(), value);
        }
        ResourceBinding::Editable => {
            anyhow::ensure!(
                session.is_editable(),
                "document {} was not opened for editing",
                session.id
            );
            let path = session
                .resource
                .file_path()
                .ok_or_else(|| anyhow::anyhow!("attachments are immutable; open a file target"))?;
            input.insert("path".to_string(), Value::String(path.to_string()));
        }
    }
    Ok(())
}

pub(super) fn validate_arguments(
    operation: &str,
    schema: &Value,
    arguments: &Value,
) -> anyhow::Result<()> {
    if let Some(error) = tool_input_schema_error(
        schema,
        arguments,
        &format!("document_execute.{operation}.arguments"),
    ) {
        anyhow::bail!(error);
    }
    Ok(())
}

pub(super) async fn execute_backend(
    call_id: Uuid,
    operation: &str,
    input: Map<String, Value>,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let mut result = execute_spreadsheet_backend(call_id, Value::Object(input), ctx).await?;
    if let Some(metadata) = result.metadata.as_object_mut() {
        metadata.insert(
            "toolName".to_string(),
            Value::String("document_execute".to_string()),
        );
        metadata.insert(
            "operation".to_string(),
            Value::String(operation.to_string()),
        );
        metadata.remove("action");
    }
    Ok(result)
}

pub(super) fn handlers() -> Vec<Box<dyn DocumentOperationHandler>> {
    use OperationEffect::{FileMutation, Observation};
    use ResourceBinding::{Editable, Read};

    vec![
        Box::new(BackendHandler::new(
            "list_sheets",
            "List worksheet names, kinds, and visibility.",
            Observation,
            Read,
            schema::<EmptyArguments>,
        )),
        Box::new(BackendHandler::new(
            "read_range",
            "Read one bounded rectangular range.",
            Observation,
            Read,
            schema::<ReadRangeArguments>,
        )),
        Box::new(BackendHandler::new(
            "read_ranges",
            "Read several bounded ranges in one operation.",
            Observation,
            Read,
            schema::<ReadRangesArguments>,
        )),
        Box::new(BackendHandler::new(
            "read_rows",
            "Read a counted row-oriented rectangle.",
            Observation,
            Read,
            schema::<ReadRowsArguments>,
        )),
        Box::new(BackendHandler::new(
            "read_columns",
            "Read a counted column-oriented rectangle.",
            Observation,
            Read,
            schema::<ReadRowsArguments>,
        )),
        Box::new(BackendHandler::new(
            "find",
            "Find matching cell values or formulas.",
            Observation,
            Read,
            schema::<FindArguments>,
        )),
        Box::new(BackendHandler::new(
            "validate",
            "Validate workbook structure, headers, and populated cell counts.",
            Observation,
            Read,
            schema::<ValidateArguments>,
        )),
        Box::new(BackendHandler::new(
            "export_delimited",
            "Export a sheet or range to CSV or TSV.",
            FileMutation,
            Read,
            schema::<ExportDelimitedArguments>,
        )),
        Box::new(BackendHandler::new(
            "write",
            "Create or update sheets and individual cells in this spreadsheet.",
            FileMutation,
            Editable,
            schema::<WriteArguments>,
        )),
        Box::new(BackendHandler::new(
            "write_rows",
            "Write a rectangular set of rows into this spreadsheet.",
            FileMutation,
            Editable,
            schema::<WriteRowsArguments>,
        )),
        Box::new(BackendHandler::new(
            "write_columns",
            "Write a rectangular set of columns into this spreadsheet.",
            FileMutation,
            Editable,
            schema::<WriteColumnsArguments>,
        )),
        Box::new(CopyHandler { name: "copy_rows" }),
        Box::new(CopyHandler {
            name: "copy_columns",
        }),
    ]
}
