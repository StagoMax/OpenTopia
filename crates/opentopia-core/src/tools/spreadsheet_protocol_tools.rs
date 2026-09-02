//! Progressive spreadsheet tool protocol.
//!
//! The public surface remains small while the stable execution schema is
//! supplied only after `spreadsheet_describe` is called. Describe can return
//! focused guidance for selected operations without changing the contract that
//! was already exposed. Execution delegates to the established
//! `SpreadsheetTool`, keeping one validation and staging path for both the
//! modern protocol and legacy calls.

use super::attachment_tool::stored_attachment_read_path;
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
use crate::provider::{ProviderToolContractLoad, PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY};
use crate::runtime_capability::OFFICE_PYTHON_EXECUTABLE_ENV;
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
    /// Use `workbook` for XLS/XLSX/ODS files. Use `delimited` only for CSV/TSV text.
    #[serde(default)]
    inspection: Option<SpreadsheetInspection>,
    /// CSV/TSV parser format. Omit for workbook inspection; this is not the workbook file type.
    #[serde(default, rename = "delimitedFormat", alias = "format")]
    #[schemars(rename = "delimitedFormat")]
    delimited_format: Option<DelimitedFormat>,
    /// Number of CSV/TSV rows to sample. Omit for workbook inspection.
    #[serde(default)]
    #[schemars(range(min = 1, max = 20))]
    sample_rows: Option<usize>,
    /// Strip trailing tab separators while reading TSV text. Omit for workbook inspection.
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
        "Stage 1 of spreadsheet work: bind and inspect one file path or immutable user attachment. Relative and absolute paths are governed by the active filesystem authority. Use it before choosing a detailed read or mutation operation."
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
            OfficeResourceRef::File { path } => {
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
            if let Some(format) = input.delimited_format {
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
    /// Existing workbook/data source. Omit only when describing creation of a new workbook.
    #[serde(default)]
    resource: Option<OfficeResourceRef>,
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
        "Stage 2 of spreadsheet work: expose the stable spreadsheet_execute tool and return exact backend constraints for selected operations. Call after spreadsheet_inspect for existing inputs; omit resource only when selecting write to create a new workbook."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(
            input
                .resource
                .as_ref()
                .map(OfficeResourceRef::resource_key)
                .into_iter()
                .collect(),
        )
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
        let mut selected_operations = Vec::with_capacity(operations.len());
        for operation in operations {
            if !selected_operations.contains(&operation) {
                selected_operations.push(operation);
            }
        }
        anyhow::ensure!(
            resource.is_some()
                || selected_operations
                    .iter()
                    .all(|operation| *operation == SpreadsheetProtocolOperation::Write),
            "resource is required unless every selected operation is write"
        );
        let resource_descriptor = resource
            .as_ref()
            .map(OfficeResourceRef::descriptor)
            .unwrap_or_else(|| {
                json!({
                    "kind": "newWorkbook",
                    "backend": "offlineFile"
                })
            });
        let contracts = selected_operations
            .iter()
            .copied()
            .map(|operation| -> anyhow::Result<Value> {
                let contract = SpreadsheetOperationContract::new(operation)?;
                let primary_binding = if operation.is_mutation() {
                    operation.primary_binding()
                } else {
                    resource
                        .as_ref()
                        .expect("observation operation requires a resource")
                        .read_binding_key()
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
        // Tool disclosure is progressive, but the disclosed tool contract is
        // stable. A later describe call may request different operation docs;
        // it must not revoke operations that the model learned earlier.
        let loaded_execute_schema = spreadsheet_execute_contract_schema()?;
        let mut metadata = Map::new();
        metadata.insert("success".to_string(), Value::Bool(true));
        metadata.insert(
            "protocol".to_string(),
            Value::String("spreadsheet/v2".to_string()),
        );
        metadata.insert("resource".to_string(), payload["resource"].clone());
        metadata.insert(
            PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY.to_string(),
            serde_json::to_value(vec![ProviderToolContractLoad::new(
                "spreadsheet_execute",
                loaded_execute_schema,
            )])?,
        );
        Ok(ToolResult::text(
            call_id,
            serde_json::to_string_pretty(&payload)?,
            Value::Object(metadata),
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

fn spreadsheet_execute_contract_schema() -> anyhow::Result<Value> {
    // Use the required resource type rather than SpreadsheetExecuteInput's
    // `Option` projection; read/mutation branches may make the property
    // optional, but an explicitly supplied resource is never null.
    let resource_schema = SpreadsheetInspectTool
        .schema()
        .pointer("/properties/resource")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("spreadsheet resource schema is missing"))?;
    let branches = SpreadsheetProtocolOperation::ALL
        .iter()
        .copied()
        .map(|operation| -> anyhow::Result<Value> {
            let arguments_schema = SpreadsheetOperationContract::new(operation)?
                .arguments_schema()
                .clone();
            let required = if operation == SpreadsheetProtocolOperation::Write {
                json!(["operation", "arguments"])
            } else {
                json!(["resource", "operation", "arguments"])
            };
            Ok(json!({
                "type": "object",
                "properties": {
                    "resource": resource_schema,
                    "operation": {
                        "type": "string",
                        "enum": [operation.legacy_action()]
                    },
                    "arguments": arguments_schema
                },
                "required": required,
                "additionalProperties": false
            }))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if branches.len() == 1 {
        Ok(branches.into_iter().next().expect("one schema branch"))
    } else {
        Ok(json!({
            "type": "object",
            "oneOf": branches
        }))
    }
}

#[async_trait]
impl TypedTool for SpreadsheetExecuteTool {
    type Input = SpreadsheetExecuteInput;

    fn name(&self) -> &str {
        "spreadsheet_execute"
    }

    fn description(&self) -> &str {
        "Stage 3 of spreadsheet work: execute any supported operation using the exact argument guidance returned by spreadsheet_describe. Its tool contract stays stable across repeated describe calls; mutations remain approval-governed and atomic."
    }

    fn provider_contract_loader(&self) -> Option<&str> {
        Some("spreadsheet_describe")
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
                OfficeResourceRef::File { path } => Some(vec![PathBuf::from(path)]),
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
                let path = match &resource {
                    OfficeResourceRef::File { .. } => PathBuf::from(resource.offline_path()?),
                    OfficeResourceRef::Attachment { attachment_id } => {
                        stored_attachment_read_path(&ctx, *attachment_id)?
                    }
                };
                arguments.insert(
                    binding.to_string(),
                    Value::String(path.to_string_lossy().into_owned()),
                );
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
    if resource["kind"] == "newWorkbook" {
        notes.push(
            "No input resource is bound; outputPath creates the new workbook under the active filesystem authority."
                .to_string(),
        );
    } else if operation.is_mutation() {
        notes.push(
            "The resource is a read input; outputPath is a separate destination governed by the active filesystem authority. The input is never modified unless outputPath intentionally resolves to the same writable file. Mutations are atomic."
                .to_string(),
        );
    }
    if matches!(operation, SpreadsheetProtocolOperation::FillTemplate) {
        notes.push(
            "resource binds templatePath; dataPath is a separate readable CSV/TSV source governed by the active filesystem authority."
                .to_string(),
        );
    }
    if matches!(operation, SpreadsheetProtocolOperation::TransferRows) {
        notes.push(
            "resource binds sourcePath; templatePath is a separate readable XLSX template. Filtering, projection, constants, and value transforms execute inside the spreadsheet backend and only a summary is returned."
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
            "python": runtime.python_version,
            "openpyxl": runtime.openpyxl_version,
            "source": runtime.source,
            "shell": {
                "executableEnvironmentVariable": OFFICE_PYTHON_EXECUTABLE_ENV,
                "note": "The host projects this runtime and its filesystem capability into shell executions. Resolve the executable from the environment variable instead of copying an internal installation path."
            }
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

#[cfg(test)]
mod runtime_projection_tests {
    use super::*;
    use crate::office_runtime::{
        ManagedOfficeRuntimeStatus, OfficePythonRuntime, OfficeRuntimeSource, OfficeRuntimeStatus,
    };

    #[test]
    fn model_runtime_status_exposes_a_logical_shell_launcher_not_internal_paths() {
        let status = OfficeRuntimeStatus {
            runtime: Some(OfficePythonRuntime {
                executable: PathBuf::from("internal/python/python.exe"),
                root: PathBuf::from("internal"),
                runtime_version: "office-test".to_string(),
                python_version: "3.12.14".to_string(),
                openpyxl_version: "3.1.5".to_string(),
                source: OfficeRuntimeSource::Managed,
            }),
            managed_version: "office-test".to_string(),
            managed_status: ManagedOfficeRuntimeStatus::Ready,
            managed_error: None,
        };

        let projected = runtime_status_json(status);
        let runtime = &projected["runtime"];
        assert_eq!(
            runtime["shell"]["executableEnvironmentVariable"],
            OFFICE_PYTHON_EXECUTABLE_ENV
        );
        assert!(runtime.get("root").is_none());
        assert!(runtime.get("executable").is_none());
    }
}
