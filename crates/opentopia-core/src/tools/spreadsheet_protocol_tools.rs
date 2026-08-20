//! Progressive spreadsheet tool protocol.
//!
//! The public surface remains small while the precise operation schema is
//! supplied only after `spreadsheet_describe` selects an operation. Execution
//! delegates to the established `SpreadsheetTool`, keeping one validation and
//! staging path for both the modern protocol and legacy calls.

use super::spreadsheet_protocol_contract::{
    SpreadsheetOperationContract, SpreadsheetProtocolOperation,
};
use super::{
    tool_resource_key, OfficeResourceRef, SpreadsheetTool, Tool, ToolExecutionPolicy,
    ToolInvocationContext, ToolSideEffect, TypedTool,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ToolCall, ToolResult};
use crate::office_runtime::OfficeRuntime;
use crate::spreadsheet::DelimitedFormat;
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
        "Stage 1 of spreadsheet work: bind and inspect one workspace file or immutable user attachment. Use it before choosing a detailed read or mutation operation."
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
            OfficeResourceRef::Attachment { .. } => ToolExecutionIntent::observation([]),
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SpreadsheetDescribeInput {
    resource: OfficeResourceRef,
    #[schemars(length(min = 1, max = 8))]
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
        let SpreadsheetDescribeInput {
            resource,
            operations,
        } = input;
        anyhow::ensure!(
            resource.supports_mutation()
                || operations.iter().all(|operation| !operation.is_mutation()),
            "attachment resources are immutable; request observation contracts only, or use a workspaceFile resource for mutations"
        );
        let resource_descriptor = resource.descriptor();
        let contracts = operations
            .into_iter()
            .map(|operation| -> anyhow::Result<Value> {
                let contract = SpreadsheetOperationContract::new(operation)?;
                let primary_binding = if operation.is_mutation() {
                    operation.primary_binding()
                } else {
                    resource.read_binding_key()
                };
                Ok(json!({
                    "operation": operation.legacy_action(),
                    "kind": if operation.is_mutation() { "mutation" } else { "observation" },
                    "primaryResourceBinding": primary_binding,
                    "argumentsSchema": contract.arguments_schema(),
                    "notes": operation_notes(operation, &resource_descriptor)
                }))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let runtime = OfficeRuntime::shared().status();
        let payload = json!({
            "protocol": "spreadsheet/v2",
            "resource": resource_descriptor,
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
    #[schemars(with = "std::collections::BTreeMap<String, serde_json::Value>")]
    arguments: Map<String, Value>,
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
                OfficeResourceRef::Attachment { .. } => None,
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
        let SpreadsheetExecuteInput {
            resource,
            operation,
            arguments,
        } = input;
        let contract = SpreadsheetOperationContract::new(operation)?;
        let arguments_value = Value::Object(arguments);
        contract.validate_arguments(&arguments_value)?;
        let Value::Object(mut arguments) = arguments_value else {
            unreachable!("spreadsheet execute arguments are constructed as an object")
        };

        if let Some(resource) = resource {
            let binding = operation.primary_binding();
            if operation.is_mutation() {
                let path = resource.offline_path()?;
                arguments.insert(binding.to_string(), Value::String(path.to_string()));
            } else {
                let (key, value) = resource.read_binding()?;
                arguments.insert(key.to_string(), value);
            }
        } else {
            anyhow::ensure!(
                operation == SpreadsheetProtocolOperation::Write,
                "resource is required except when creating a new workbook with operation=write"
            );
        }
        arguments.insert(
            "action".to_string(),
            Value::String(operation.legacy_action().to_string()),
        );
        execute_legacy_spreadsheet(call_id, Value::Object(arguments), ctx).await
    }
}

fn output_path_from_arguments(arguments: &Map<String, Value>) -> Option<&str> {
    arguments
        .get("outputPath")
        .or_else(|| arguments.get("path"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn argument_read_paths(arguments: &Map<String, Value>) -> Vec<&str> {
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
    let runtime = status.runtime.as_ref();
    json!({
        "available": runtime.is_some(),
        "managedVersion": status.managed_version,
        "managedStatus": status.managed_status,
        "reason": status.managed_error,
        "runtime": runtime.map(|runtime| json!({
            "version": runtime.runtime_version,
            "root": runtime.root,
            "executable": runtime.executable,
            "python": runtime.python_version,
            "openpyxl": runtime.openpyxl_version,
            "source": runtime.source,
        })),
    })
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
