//! Progressive spreadsheet tool protocol.
//!
//! The public surface remains small while the precise operation schema is
//! supplied only after `spreadsheet_describe` selects an operation. Execution
//! delegates to the established `SpreadsheetTool`, keeping one validation and
//! staging path for both the modern protocol and legacy calls.

use super::{
    tool_resource_key, OfficeResourceRef, SpreadsheetTool, Tool, ToolExecutionPolicy,
    ToolInvocationContext, ToolSideEffect, TypedTool,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ToolCall, ToolResult};
use crate::office_runtime::OfficeRuntime;
use crate::spreadsheet::DelimitedFormat;
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SpreadsheetInspection {
    Workbook,
    Delimited,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetInspectInput {
    resource: OfficeResourceRef,
    #[serde(default)]
    inspection: Option<SpreadsheetInspection>,
    #[serde(default)]
    format: Option<DelimitedFormat>,
    #[serde(default)]
    #[schemars(range(min = 1, max = 20))]
    sample_rows: Option<usize>,
    #[serde(default)]
    rstrip_tabs: bool,
}

pub struct SpreadsheetInspectTool;

#[async_trait]
impl TypedTool for SpreadsheetInspectTool {
    type Input = SpreadsheetInspectInput;

    fn name(&self) -> &str {
        "spreadsheet_inspect"
    }

    fn description(&self) -> &str {
        "Stage 1 of spreadsheet work: bind and inspect one workspace file or user attachment. Use it before choosing a detailed read or mutation operation. liveSession is reserved for a future Excel integration and is reported as unavailable, never treated as a local path."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![input.resource.resource_key()])
    }

    fn execution_intent(
        &self,
        input: &Self::Input,
        _workspace_root: &std::path::Path,
    ) -> ToolExecutionIntent {
        match &input.resource {
            OfficeResourceRef::WorkspaceFile { path } => {
                ToolExecutionIntent::observation([PathBuf::from(path)])
            }
            OfficeResourceRef::Attachment { .. } | OfficeResourceRef::LiveSession { .. } => {
                ToolExecutionIntent::observation([])
            }
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        input.resource.ensure_available()?;
        let (binding_key, binding_value) = input.resource.read_binding()?;
        let inspection = input.inspection.unwrap_or(SpreadsheetInspection::Workbook);
        let mut legacy = Map::new();
        legacy.insert(
            "action".to_string(),
            Value::String(
                match inspection {
                    SpreadsheetInspection::Workbook => "inspect",
                    SpreadsheetInspection::Delimited => "inspect_delimited",
                }
                .to_string(),
            ),
        );
        legacy.insert(binding_key.to_string(), binding_value);
        if matches!(inspection, SpreadsheetInspection::Delimited) {
            if let Some(format) = input.format {
                legacy.insert("format".to_string(), serde_json::to_value(format)?);
            }
            if let Some(sample_rows) = input.sample_rows {
                legacy.insert("sampleRows".to_string(), json!(sample_rows));
            }
            if input.rstrip_tabs {
                legacy.insert("rstripTabs".to_string(), Value::Bool(true));
            }
        }
        execute_legacy_spreadsheet(call_id, Value::Object(legacy), ctx).await
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SpreadsheetProtocolOperation {
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

impl SpreadsheetProtocolOperation {
    fn legacy_action(self) -> &'static str {
        match self {
            Self::ListSheets => "list_sheets",
            Self::ReadRange => "read_range",
            Self::ReadRanges => "read_ranges",
            Self::ReadRows => "read_rows",
            Self::ReadColumns => "read_columns",
            Self::Find => "find",
            Self::FilterRows => "filter_rows",
            Self::Validate => "validate",
            Self::FillTemplate => "fill_template",
            Self::ExportDelimited => "export_delimited",
            Self::Write => "write",
            Self::WriteRows => "write_rows",
            Self::WriteColumns => "write_columns",
            Self::CopyRows => "copy_rows",
            Self::CopyColumns => "copy_columns",
            Self::Batch => "batch",
        }
    }

    fn is_mutation(self) -> bool {
        !matches!(
            self,
            Self::ListSheets
                | Self::ReadRange
                | Self::ReadRanges
                | Self::ReadRows
                | Self::ReadColumns
                | Self::Find
                | Self::FilterRows
                | Self::Validate
        )
    }

    fn primary_binding(self) -> Option<&'static str> {
        match self {
            Self::ListSheets
            | Self::ReadRange
            | Self::ReadRanges
            | Self::ReadRows
            | Self::ReadColumns
            | Self::Find
            | Self::FilterRows
            | Self::Validate
            | Self::ExportDelimited => Some("path"),
            Self::FillTemplate => Some("templatePath"),
            Self::Write | Self::WriteRows | Self::WriteColumns | Self::Batch => Some("sourcePath"),
            Self::CopyRows | Self::CopyColumns => Some("from"),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetDescribeInput {
    resource: OfficeResourceRef,
    #[serde(default)]
    operations: Vec<SpreadsheetProtocolOperation>,
}

pub struct SpreadsheetDescribeTool;

#[async_trait]
impl TypedTool for SpreadsheetDescribeTool {
    type Input = SpreadsheetDescribeInput;

    fn name(&self) -> &str {
        "spreadsheet_describe"
    }

    fn description(&self) -> &str {
        "Stage 2 of spreadsheet work: return exact argument schemas and backend constraints for selected operations. Call after spreadsheet_inspect, then pass the returned schema's fields unchanged in spreadsheet_execute.arguments."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![input.resource.resource_key()])
    }

    fn execution_intent(
        &self,
        _input: &Self::Input,
        _workspace_root: &std::path::Path,
    ) -> ToolExecutionIntent {
        ToolExecutionIntent::observation([])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        input.resource.ensure_available()?;
        let operations = if input.operations.is_empty() {
            all_protocol_operations()
        } else {
            input.operations
        };
        let resource = input.resource.descriptor();
        let contracts = operations
            .into_iter()
            .map(|operation| {
                let binding = operation.primary_binding();
                json!({
                    "operation": operation.legacy_action(),
                    "kind": if operation.is_mutation() { "mutation" } else { "observation" },
                    "primaryResourceBinding": binding,
                    "argumentsSchema": legacy_action_schema(operation.legacy_action()),
                    "notes": operation_notes(operation, &resource)
                })
            })
            .collect::<Vec<_>>();
        let runtime = OfficeRuntime::shared().status();
        let payload = json!({
            "protocol": "spreadsheet/v2",
            "resource": resource,
            "operations": contracts,
            "managedRuntime": runtime_status_json(runtime),
            "execution": {
                "tool": "spreadsheet_execute",
                "rule": "Do not include action or the primary resource field in arguments; the protocol binds it from resource."
            }
        });
        Ok(ToolResult::text(
            call_id,
            serde_json::to_string_pretty(&payload)?,
            json!({ "success": true, "protocol": "spreadsheet/v2", "resource": payload["resource"] }),
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetExecuteInput {
    /// The primary existing workbook/data source. Omit only when creating a new workbook.
    #[serde(default)]
    resource: Option<OfficeResourceRef>,
    operation: SpreadsheetProtocolOperation,
    /// Fields from spreadsheet_describe.argumentsSchema, excluding action and the primary resource field.
    arguments: Value,
}

pub struct SpreadsheetExecuteTool;

#[async_trait]
impl TypedTool for SpreadsheetExecuteTool {
    type Input = SpreadsheetExecuteInput;

    fn name(&self) -> &str {
        "spreadsheet_execute"
    }

    fn description(&self) -> &str {
        "Stage 3 of spreadsheet work: execute a selected operation using the exact argument contract returned by spreadsheet_describe. Mutations remain approval-governed and atomic; observations use the same resource contract."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let mut resource_keys = vec![input
            .resource
            .as_ref()
            .map(OfficeResourceRef::resource_key)
            .unwrap_or_else(|| tool_resource_key("file", "*"))];
        if input.operation.is_mutation() {
            if let Some(output) = output_path_from_arguments(&input.arguments) {
                resource_keys.push(tool_resource_key("file", output));
            }
            resource_keys.sort();
            resource_keys.dedup();
        }
        ToolExecutionPolicy {
            read_only: !input.operation.is_mutation(),
            idempotent: !input.operation.is_mutation(),
            parallel_safe: true,
            side_effect: if input.operation.is_mutation() {
                ToolSideEffect::WorkspaceWrite
            } else {
                ToolSideEffect::None
            },
            resource_keys,
        }
    }

    fn execution_intent(
        &self,
        input: &Self::Input,
        _workspace_root: &std::path::Path,
    ) -> ToolExecutionIntent {
        let mut reads = input
            .resource
            .as_ref()
            .and_then(|resource| match resource {
                OfficeResourceRef::WorkspaceFile { path } => Some(vec![PathBuf::from(path)]),
                OfficeResourceRef::Attachment { .. } | OfficeResourceRef::LiveSession { .. } => {
                    None
                }
            })
            .unwrap_or_default();
        reads.extend(
            argument_read_paths(&input.arguments)
                .into_iter()
                .map(PathBuf::from),
        );
        if input.operation.is_mutation() {
            ToolExecutionIntent::workspace_mutation(
                output_path_from_arguments(&input.arguments).map(PathBuf::from),
            )
            .with_read_paths(reads)
        } else {
            ToolExecutionIntent::observation(reads)
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let mut arguments = input
            .arguments
            .as_object()
            .cloned()
            .context("spreadsheet_execute.arguments must be an object")?;
        anyhow::ensure!(
            arguments.remove("action").is_none(),
            "spreadsheet_execute.arguments must not include action"
        );
        if let Some(resource) = input.resource {
            resource.ensure_available()?;
            let binding = input
                .operation
                .primary_binding()
                .expect("every protocol operation has a primary binding");
            anyhow::ensure!(
                arguments.get(binding).is_none(),
                "spreadsheet_execute.arguments must not include {binding}; it is bound from resource"
            );
            if input.operation.is_mutation() {
                let path = resource.offline_path()?;
                arguments.insert(binding.to_string(), Value::String(path.to_string()));
            } else {
                let (key, value) = resource.read_binding()?;
                anyhow::ensure!(
                    key == binding || arguments.get(key).is_none(),
                    "spreadsheet_execute.arguments must not include {key}; it is bound from resource"
                );
                arguments.insert(key.to_string(), value);
            }
        } else {
            anyhow::ensure!(
                input.operation == SpreadsheetProtocolOperation::Write,
                "resource is required except when creating a new workbook with operation=write"
            );
        }
        arguments.insert(
            "action".to_string(),
            Value::String(input.operation.legacy_action().to_string()),
        );
        execute_legacy_spreadsheet(call_id, Value::Object(arguments), ctx).await
    }
}

fn all_protocol_operations() -> Vec<SpreadsheetProtocolOperation> {
    vec![
        SpreadsheetProtocolOperation::ListSheets,
        SpreadsheetProtocolOperation::ReadRange,
        SpreadsheetProtocolOperation::ReadRanges,
        SpreadsheetProtocolOperation::ReadRows,
        SpreadsheetProtocolOperation::ReadColumns,
        SpreadsheetProtocolOperation::Find,
        SpreadsheetProtocolOperation::FilterRows,
        SpreadsheetProtocolOperation::Validate,
        SpreadsheetProtocolOperation::FillTemplate,
        SpreadsheetProtocolOperation::ExportDelimited,
        SpreadsheetProtocolOperation::Write,
        SpreadsheetProtocolOperation::WriteRows,
        SpreadsheetProtocolOperation::WriteColumns,
        SpreadsheetProtocolOperation::CopyRows,
        SpreadsheetProtocolOperation::CopyColumns,
        SpreadsheetProtocolOperation::Batch,
    ]
}

fn output_path_from_arguments(arguments: &Value) -> Option<&str> {
    arguments
        .get("outputPath")
        .or_else(|| arguments.get("path"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn argument_read_paths(arguments: &Value) -> Vec<&str> {
    let mut paths = ["dataPath", "templatePath", "sourcePath", "path", "from"]
        .into_iter()
        .filter_map(|key| arguments.get(key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>();
    if let Some(operations) = arguments.get("operations").and_then(Value::as_array) {
        paths.extend(
            operations
                .iter()
                .filter_map(|operation| operation.get("sourcePath").and_then(Value::as_str))
                .filter(|value| !value.trim().is_empty()),
        );
    }
    paths
}

fn legacy_action_schema(action: &str) -> Value {
    let schema = SpreadsheetTool.schema();
    schema["oneOf"]
        .as_array()
        .and_then(|branches| {
            branches.iter().find(|branch| {
                branch["properties"]["action"]["enum"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(action)))
            })
        })
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object", "description": "No schema was registered for this operation." }))
}

fn operation_notes(operation: SpreadsheetProtocolOperation, resource: &Value) -> Value {
    let backend = resource["backend"].as_str().unwrap_or("unknown");
    let mut notes = vec![format!("Resource backend: {backend}.")];
    if operation.is_mutation() {
        notes.push(
            "outputPath must be a workspace-relative destination path; mutations are atomic."
                .to_string(),
        );
    }
    if matches!(operation, SpreadsheetProtocolOperation::FillTemplate) {
        notes.push(
            "resource binds templatePath; dataPath remains a workspace-relative CSV/TSV source."
                .to_string(),
        );
    }
    Value::Array(notes.into_iter().map(Value::String).collect())
}

fn runtime_status_json(status: crate::office_runtime::OfficeRuntimeStatus) -> Value {
    match status {
        crate::office_runtime::OfficeRuntimeStatus::Ready {
            version,
            root,
            openpyxl_version,
        } => {
            json!({ "available": true, "version": version, "root": root, "openpyxl": openpyxl_version })
        }
        crate::office_runtime::OfficeRuntimeStatus::LegacyOverride { executable } => {
            json!({ "available": true, "source": "legacyOverride", "executable": executable })
        }
        crate::office_runtime::OfficeRuntimeStatus::Unavailable { reason } => {
            json!({ "available": false, "reason": reason })
        }
    }
}

async fn execute_legacy_spreadsheet(
    call_id: Uuid,
    input: Value,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    SpreadsheetTool
        .execute(
            ToolCall {
                id: call_id,
                name: "spreadsheet".to_string(),
                input,
            },
            ctx,
        )
        .await
}
