//! Format-neutral progressive document protocol.
//!
//! The model sees three stable tools. Format adapters contribute atomic
//! operations behind the protocol; no format-specific state is added to the
//! agent loop.

use super::document_session::{
    disclose_operations, get_document, insert_document, require_disclosed_operation, DocumentKind,
    DocumentOpenMode,
};
use super::spreadsheet_operations::{available_operation_names, handler_for, OperationEffect};
use super::spreadsheet_tool::execute_spreadsheet_backend;
use super::{
    normalize_workspace_path, tool_resource_key, DocumentResourceRef, Tool, ToolExecutionPolicy,
    ToolInvocationContext, ToolSideEffect, TypedTool,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::{ModelContentPart, ToolResult};
use crate::provider::{ProviderToolContractLoad, PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY};
use crate::sandbox::SandboxMode;
use crate::spreadsheet::SpreadsheetFileFormat;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DocumentOpenInput {
    resource: DocumentResourceRef,
    #[serde(default)]
    mode: DocumentOpenMode,
}

pub struct DocumentOpenTool;

#[async_trait]
impl TypedTool for DocumentOpenTool {
    type Input = DocumentOpenInput;

    fn name(&self) -> &str {
        "document_open"
    }

    fn description(&self) -> &str {
        "Open one file or immutable attachment as a thread-scoped document handle. Use read for sources, edit for an existing writable target, or create for a new file path."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: true,
            idempotent: false,
            parallel_safe: true,
            side_effect: ToolSideEffect::SessionMutation,
            resource_keys: vec![input.resource.resource_key()],
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        match input.mode {
            DocumentOpenMode::Read => match &input.resource {
                DocumentResourceRef::File { path } => {
                    ToolExecutionIntent::observation([PathBuf::from(path)])
                }
                DocumentResourceRef::Attachment { .. } => ToolExecutionIntent::observation([]),
            },
            DocumentOpenMode::Edit => match &input.resource {
                DocumentResourceRef::File { path } => ToolExecutionIntent::workspace_mutation([])
                    .with_read_paths([PathBuf::from(path)]),
                DocumentResourceRef::Attachment { .. } => {
                    ToolExecutionIntent::workspace_mutation([])
                }
            },
            DocumentOpenMode::Create => ToolExecutionIntent::workspace_mutation([]),
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        validate_open_mode(&input, &ctx)?;
        let inspection = if input.mode == DocumentOpenMode::Create {
            None
        } else {
            let (binding, value) = input.resource.read_binding()?;
            let mut backend_input = Map::new();
            backend_input.insert("action".to_string(), json!("inspect"));
            backend_input.insert(binding.to_string(), value);
            let mut result =
                execute_spreadsheet_backend(call_id, Value::Object(backend_input), ctx.clone())
                    .await?;
            if result.metadata["success"] != true {
                relabel_result(&mut result, "document_open", None, None);
                return Ok(result);
            }
            result.content.iter().find_map(|part| match part {
                ModelContentPart::Json { value } => Some(value.clone()),
                _ => None,
            })
        };
        let session = insert_document(
            ctx.thread_id,
            DocumentKind::Spreadsheet,
            input.mode,
            input.resource,
        )?;
        let payload = json!({
            "documentId": session.id,
            "documentType": session.kind.as_str(),
            "mode": session.mode,
            "resource": session.resource.descriptor(),
            "writable": session.is_editable(),
            "inspection": inspection,
            "availableOperations": available_operation_names(&session),
        });
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&payload)?,
            content: vec![ModelContentPart::json(payload)],
            metadata: json!({
                "toolName": "document_open",
                "success": true,
                "documentId": session.id,
                "documentType": session.kind.as_str(),
            }),
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DocumentGetOperationSchemasInput {
    document_id: Uuid,
    #[schemars(length(min = 1, max = 100))]
    operations: Vec<String>,
}

pub struct DocumentGetOperationSchemasTool;

#[async_trait]
impl TypedTool for DocumentGetOperationSchemasTool {
    type Input = DocumentGetOperationSchemasInput;

    fn name(&self) -> &str {
        "document_get_operation_schemas"
    }

    fn description(&self) -> &str {
        "Load exact argument schemas for selected operations on an open document. Up to 100 operation names may be requested together."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key(
            "document",
            &input.document_id.to_string(),
        )])
    }

    fn execution_intent(
        &self,
        _input: &Self::Input,
        _workspace_root: &Path,
    ) -> ToolExecutionIntent {
        ToolExecutionIntent::observation([])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let session = get_document(input.document_id, ctx.thread_id)?;
        let available = available_operation_names(&session)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut names = Vec::new();
        let mut contracts = Vec::new();
        for requested in input.operations {
            let operation = requested.trim();
            anyhow::ensure!(!operation.is_empty(), "operation names must not be empty");
            if names.iter().any(|name| name == operation) {
                continue;
            }
            anyhow::ensure!(
                available.contains(operation),
                "operation `{operation}` is not available for document {} in {:?} mode",
                session.id,
                session.mode
            );
            let handler = handler_for(session.kind, operation)
                .ok_or_else(|| anyhow::anyhow!("unknown operation `{operation}`"))?;
            names.push(operation.to_string());
            contracts.push(json!({
                "operation": handler.name(),
                "version": handler.version(),
                "effect": effect_name(handler.effect()),
                "requiresEditableDocument": handler.requires_editable_document(),
                "description": handler.description(),
                "argumentsSchema": handler.arguments_schema(),
            }));
        }
        disclose_operations(session.id, ctx.thread_id, names.clone())?;
        let payload = json!({
            "protocol": "document/v1",
            "documentId": session.id,
            "documentType": session.kind.as_str(),
            "operations": contracts,
            "execution": {
                "tool": "document_execute",
                "envelope": ["documentId", "operation", "arguments"]
            }
        });
        let mut metadata = Map::new();
        metadata.insert(
            "toolName".to_string(),
            json!("document_get_operation_schemas"),
        );
        metadata.insert("success".to_string(), Value::Bool(true));
        metadata.insert("documentId".to_string(), json!(session.id));
        metadata.insert(
            PROVIDER_TOOL_CONTRACT_LOADS_METADATA_KEY.to_string(),
            serde_json::to_value(vec![ProviderToolContractLoad::new(
                "document_execute",
                DocumentExecuteTool.schema(),
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
pub(super) struct DocumentExecuteInput {
    document_id: Uuid,
    operation: String,
    #[schemars(with = "std::collections::BTreeMap<String, serde_json::Value>")]
    arguments: Map<String, Value>,
}

pub struct DocumentExecuteTool;

#[async_trait]
impl TypedTool for DocumentExecuteTool {
    type Input = DocumentExecuteInput;

    fn name(&self) -> &str {
        "document_execute"
    }

    fn description(&self) -> &str {
        "Execute one previously loaded atomic operation against an open document. The envelope is stable; operation-specific arguments come from document_get_operation_schemas."
    }

    fn provider_contract_loader(&self) -> Option<&str> {
        Some("document_get_operation_schemas")
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let session = super::document_session::get_document_unscoped(input.document_id).ok();
        let handler = session
            .as_ref()
            .and_then(|session| handler_for(session.kind, input.operation.trim()));
        let effect = handler
            .map(|handler| handler.effect())
            .unwrap_or(OperationEffect::FileMutation);
        let mut resource_keys = session
            .as_ref()
            .map(|session| vec![session.resource.resource_key()])
            .unwrap_or_else(|| {
                vec![tool_resource_key(
                    "document",
                    &input.document_id.to_string(),
                )]
            });
        if let Some(output) = input.arguments.get("outputPath").and_then(Value::as_str) {
            resource_keys.push(tool_resource_key("file", output));
        }
        resource_keys.sort();
        resource_keys.dedup();
        ToolExecutionPolicy {
            read_only: effect == OperationEffect::Observation,
            idempotent: effect == OperationEffect::Observation,
            parallel_safe: true,
            side_effect: match effect {
                OperationEffect::Observation => ToolSideEffect::None,
                OperationEffect::SessionMutation => ToolSideEffect::SessionMutation,
                OperationEffect::FileMutation => ToolSideEffect::WorkspaceWrite,
            },
            resource_keys,
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        let session = super::document_session::get_document_unscoped(input.document_id).ok();
        let effect = session
            .as_ref()
            .and_then(|session| handler_for(session.kind, input.operation.trim()))
            .map(|handler| handler.effect())
            .unwrap_or(OperationEffect::FileMutation);
        let document_path = session
            .as_ref()
            .and_then(|session| session.resource.file_path())
            .map(PathBuf::from);
        match effect {
            OperationEffect::Observation | OperationEffect::SessionMutation => {
                ToolExecutionIntent::observation(document_path)
            }
            OperationEffect::FileMutation => {
                let output = input
                    .arguments
                    .get("outputPath")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .or(document_path);
                ToolExecutionIntent::workspace_mutation(output)
            }
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let session = get_document(input.document_id, ctx.thread_id)?;
        let operation = input.operation.trim();
        require_disclosed_operation(&session, operation)?;
        let handler = handler_for(session.kind, operation)
            .ok_or_else(|| anyhow::anyhow!("unknown operation `{operation}`"))?;
        if handler.requires_editable_document() {
            anyhow::ensure!(
                session.is_editable(),
                "document {} was opened in read mode and cannot be modified",
                session.id
            );
        }
        let mut result = handler
            .execute(call_id, &session, Value::Object(input.arguments), ctx)
            .await?;
        relabel_result(
            &mut result,
            "document_execute",
            Some(session.id),
            Some(operation),
        );
        Ok(result)
    }
}

fn validate_open_mode(
    input: &DocumentOpenInput,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<()> {
    let path = input.resource.file_path();
    if input.mode != DocumentOpenMode::Read
        && ctx
            .sandbox_config()
            .is_some_and(|config| config.sandbox_mode == SandboxMode::ReadOnly)
    {
        anyhow::bail!("the active read-only sandbox cannot open an editable document session");
    }
    match input.mode {
        DocumentOpenMode::Read => {}
        DocumentOpenMode::Edit => {
            let path = path.ok_or_else(|| {
                anyhow::anyhow!("attachments are immutable; open a file target in edit mode")
            })?;
            let logical = normalize_workspace_path(&ctx.workspace_root, path)?;
            let resolved = ctx.environment.resolve_read_path(&logical)?;
            let metadata = std::fs::metadata(&resolved).map_err(|error| {
                anyhow::anyhow!("cannot open {} for editing: {error}", resolved.display())
            })?;
            anyhow::ensure!(
                !metadata.permissions().readonly(),
                "{} is read-only; choose a writable target file",
                resolved.display()
            );
        }
        DocumentOpenMode::Create => {
            let path = path.ok_or_else(|| {
                anyhow::anyhow!("create mode requires a file path, not an attachment")
            })?;
            anyhow::ensure!(
                SpreadsheetFileFormat::from_path(Path::new(path)).is_some(),
                "create mode requires a supported spreadsheet file extension"
            );
            let logical = normalize_workspace_path(&ctx.workspace_root, path)?;
            anyhow::ensure!(
                !logical.exists(),
                "{} already exists; open it in edit mode",
                logical.display()
            );
        }
    }
    Ok(())
}

fn effect_name(effect: OperationEffect) -> &'static str {
    match effect {
        OperationEffect::Observation => "observation",
        OperationEffect::SessionMutation => "session_mutation",
        OperationEffect::FileMutation => "file_mutation",
    }
}

fn relabel_result(
    result: &mut ToolResult,
    tool_name: &str,
    document_id: Option<Uuid>,
    operation: Option<&str>,
) {
    let Some(metadata) = result.metadata.as_object_mut() else {
        return;
    };
    metadata.insert("toolName".to_string(), json!(tool_name));
    metadata.remove("action");
    if let Some(document_id) = document_id {
        metadata.insert("documentId".to_string(), json!(document_id));
    }
    if let Some(operation) = operation {
        metadata.insert("operation".to_string(), json!(operation));
    }
}
