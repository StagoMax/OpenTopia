use super::super::{
    enforce_policy_decision, enforce_read_policy, normalize_workspace_path,
    spreadsheet_tool::spreadsheet_error_result, ToolExecutionPolicy, ToolInvocationContext,
    TypedTool,
};
use super::common::{
    mutation_intent, mutation_policy, parse_row_conditions, SpreadsheetRowConditionInput,
};
use crate::execution::FileReadRequest;
use crate::execution_authorization::ToolExecutionIntent;
use crate::file_mutation::{read_optional, FileMutationBatch, PreparedFileMutation};
use crate::model::{ModelContentPart, ToolResult};
use crate::spreadsheet::{
    edit_workbook_structure, filter_rows, parse_a1_range, EditWorkbookStructureRequest,
    FilterRowsRequest, SheetVisibility, SpreadsheetFilterMatchMode, SpreadsheetFilterReturnMode,
    SpreadsheetStructureOperation, MAX_FILTER_RESULTS, MAX_INPUT_FILE_BYTES,
};
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetCopySheetInput {
    source_path: String,
    source_sheet: String,
    path: String,
    #[serde(default)]
    template: Option<String>,
    destination_sheet: String,
    #[serde(default)]
    visibility: Option<SheetVisibility>,
}

pub struct SpreadsheetCopySheetTool;

#[async_trait]
impl TypedTool for SpreadsheetCopySheetTool {
    type Input = SpreadsheetCopySheetInput;

    fn name(&self) -> &str {
        "spreadsheet_copy_sheet"
    }

    fn description(&self) -> &str {
        "Copy a worksheet directly between workbooks. Set template to rebuild an output from that workbook; reruns replace the same output path."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        mutation_policy(
            [input.source_path.clone(), input.path.clone()]
                .into_iter()
                .chain(input.template.clone()),
            [input.path.clone()],
        )
    }

    fn execution_intent(&self, input: &Self::Input, _: &Path) -> ToolExecutionIntent {
        mutation_intent(
            [input.source_path.clone(), input.path.clone()]
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
        let target = read_target_workbook(&ctx, &input.path, input.template.as_deref()).await?;
        let source = read_workbook(&ctx, &input.source_path).await?;
        execute_structure_edit(
            self.name(),
            call_id,
            target,
            vec![source],
            move |_, staged_sources| {
                vec![SpreadsheetStructureOperation::CopySheet {
                    source: staged_sources[0].clone(),
                    source_sheet: input.source_sheet,
                    destination_sheet: input.destination_sheet,
                    visibility: input.visibility,
                }]
            },
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetDeleteRowsInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    sheet: String,
    /// Rows and columns to scan in Excel A1 notation.
    range: String,
    #[schemars(length(min = 1, max = 32))]
    conditions: Vec<SpreadsheetRowConditionInput>,
    #[serde(default)]
    match_mode: Option<SpreadsheetFilterMatchMode>,
}

pub struct SpreadsheetDeleteRowsTool;

#[async_trait]
impl TypedTool for SpreadsheetDeleteRowsTool {
    type Input = SpreadsheetDeleteRowsInput;

    fn name(&self) -> &str {
        "spreadsheet_delete_rows"
    }

    fn description(&self) -> &str {
        "Delete worksheet rows matching typed conditions. Set template to rebuild a filtered output from another workbook."
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
        let target = read_target_workbook(&ctx, &input.path, input.template.as_deref()).await?;
        let range = parse_a1_range(&input.range)?;
        let conditions = parse_row_conditions(input.conditions)?;
        execute_structure_edit(
            self.name(),
            call_id,
            target,
            Vec::new(),
            move |staged_target, _| {
                let filtered = filter_rows(&FilterRowsRequest {
                    path: staged_target.to_path_buf(),
                    sheet: input.sheet.clone(),
                    range,
                    conditions,
                    match_mode: input.match_mode.unwrap_or_default(),
                    return_mode: SpreadsheetFilterReturnMode::Indices,
                    max_results: MAX_FILTER_RESULTS,
                })?;
                if filtered.truncated {
                    return Err(crate::spreadsheet::SpreadsheetError::InvalidFilter {
                        reason: format!(
                            "more than {MAX_FILTER_RESULTS} rows matched; narrow the range and repeat"
                        ),
                    });
                }
                if filtered.matched_row_indices.is_empty() {
                    return Err(crate::spreadsheet::SpreadsheetError::InvalidFilter {
                        reason: "no rows matched; the workbook was not changed".to_string(),
                    });
                }
                Ok(vec![SpreadsheetStructureOperation::DeleteRows {
                    sheet: input.sheet,
                    rows: filtered.matched_row_indices,
                }])
            },
            ctx,
        )
        .await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct SpreadsheetDeleteSheetInput {
    path: String,
    #[serde(default)]
    template: Option<String>,
    sheet: String,
}

pub struct SpreadsheetDeleteSheetTool;

#[async_trait]
impl TypedTool for SpreadsheetDeleteSheetTool {
    type Input = SpreadsheetDeleteSheetInput;

    fn name(&self) -> &str {
        "spreadsheet_delete_sheet"
    }

    fn description(&self) -> &str {
        "Delete one named worksheet from a workbook while preserving the remaining workbook."
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
        let target = read_target_workbook(&ctx, &input.path, input.template.as_deref()).await?;
        execute_structure_edit(
            self.name(),
            call_id,
            target,
            Vec::new(),
            move |_, _| vec![SpreadsheetStructureOperation::DeleteSheet { sheet: input.sheet }],
            ctx,
        )
        .await
    }
}

#[derive(Clone)]
struct StagedWorkbook {
    logical_path: PathBuf,
    staged_path: PathBuf,
    bytes: Vec<u8>,
    original: Option<Vec<u8>>,
}

async fn read_workbook(
    ctx: &ToolInvocationContext,
    requested: &str,
) -> anyhow::Result<StagedWorkbook> {
    let logical_path = normalize_workspace_path(&ctx.workspace_root, requested)?;
    enforce_read_policy(ctx, &logical_path)?;
    let resolved = ctx.environment.resolve_read_path(&logical_path)?;
    let read = ctx
        .environment
        .read_file(FileReadRequest::new(&resolved).with_max_bytes(MAX_INPUT_FILE_BYTES))
        .await?;
    Ok(StagedWorkbook {
        logical_path,
        staged_path: read.path,
        original: Some(read.bytes.clone()),
        bytes: read.bytes,
    })
}

async fn read_target_workbook(
    ctx: &ToolInvocationContext,
    requested: &str,
    template_requested: Option<&str>,
) -> anyhow::Result<StagedWorkbook> {
    let Some(template_requested) = template_requested else {
        return read_workbook(ctx, requested).await;
    };
    let logical_path = normalize_workspace_path(&ctx.workspace_root, requested)?;
    enforce_read_policy(ctx, &logical_path)?;
    let original = read_optional(ctx.environment.as_ref(), &logical_path).await?;
    let template = read_workbook(ctx, template_requested).await?;
    Ok(StagedWorkbook {
        logical_path,
        staged_path: template.staged_path,
        bytes: template.bytes,
        original,
    })
}

async fn execute_structure_edit<F, O>(
    tool_name: &str,
    call_id: Uuid,
    target: StagedWorkbook,
    sources: Vec<StagedWorkbook>,
    build_operations: F,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult>
where
    F: FnOnce(&Path, &[PathBuf]) -> O + Send + 'static,
    O: IntoStructureOperations + Send + 'static,
{
    enforce_policy_decision(ctx.policy.inspect_write(&target.logical_path), &ctx)?;
    let changed_path = target.logical_path.clone();
    let original = target.original.clone();
    let staged = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let temporary = SpreadsheetStructureStaging::new()?;
        let target_extension = target
            .staged_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("xlsx");
        let staged_target = temporary.path().join(format!("target.{target_extension}"));
        let staged_output = temporary.path().join(format!("output.{target_extension}"));
        fs::write(&staged_target, &target.bytes)
            .with_context(|| format!("failed to stage {}", target.logical_path.display()))?;
        let mut staged_sources = Vec::with_capacity(sources.len());
        for (index, source) in sources.into_iter().enumerate() {
            let extension = source
                .staged_path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("xlsx");
            let path = temporary.path().join(format!("source-{index}.{extension}"));
            fs::write(&path, source.bytes)
                .with_context(|| format!("failed to stage {}", source.logical_path.display()))?;
            staged_sources.push(path);
        }
        let operations = build_operations(&staged_target, &staged_sources).into_operations()?;
        let result = edit_workbook_structure(&EditWorkbookStructureRequest {
            source: staged_target,
            output: staged_output.clone(),
            operations,
        });
        match result {
            Ok(result) => Ok(Ok((result, fs::read(&staged_output)?))),
            Err(error) => Ok(Err(error)),
        }
    })
    .await
    .context("spreadsheet structure worker task failed")??;
    let (mut result, bytes) = match staged {
        Ok(value) => value,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
        &changed_path,
        original,
        bytes,
    )])?;
    ctx.commit_file_mutations(&batch).await?;
    result.output = changed_path.clone();
    let value = serde_json::to_value(&result)?;
    Ok(ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&value)?,
        content: vec![
            ModelContentPart::json(value),
            ModelContentPart::resource(
                changed_path.to_string_lossy(),
                Some(
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string(),
                ),
                changed_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string),
            ),
        ],
        metadata: json!({
            "toolName": tool_name,
            "success": true,
            "changedPath": changed_path
        }),
    })
}

trait IntoStructureOperations {
    fn into_operations(
        self,
    ) -> Result<Vec<SpreadsheetStructureOperation>, crate::spreadsheet::SpreadsheetError>;
}

struct SpreadsheetStructureStaging {
    root: PathBuf,
}

impl SpreadsheetStructureStaging {
    fn new() -> anyhow::Result<Self> {
        let root = std::env::temp_dir().join(format!("opentopia-xlsx-{}", Uuid::new_v4()));
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for SpreadsheetStructureStaging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl IntoStructureOperations for Vec<SpreadsheetStructureOperation> {
    fn into_operations(
        self,
    ) -> Result<Vec<SpreadsheetStructureOperation>, crate::spreadsheet::SpreadsheetError> {
        Ok(self)
    }
}

impl IntoStructureOperations
    for Result<Vec<SpreadsheetStructureOperation>, crate::spreadsheet::SpreadsheetError>
{
    fn into_operations(
        self,
    ) -> Result<Vec<SpreadsheetStructureOperation>, crate::spreadsheet::SpreadsheetError> {
        self
    }
}
