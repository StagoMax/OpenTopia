use crate::browser::{
    BrowserContent, BrowserDownloadRequest, BrowserNavigateRequest, BrowserRuntime,
    BrowserSelector, BrowserSessionId, BrowserTypeRequest, BrowserWaitCondition,
    BrowserWaitRequest,
};
use crate::execution::{
    ExecRequest, ExecutionContext, ExecutionEnvironment, FileReadRequest, FileWriteRequest,
    LocalExecutionEnvironment,
};
use crate::mcp::{McpCallResult, McpToolDescriptor};
use crate::mcp_host::McpExtensionHost;
use crate::model::{
    ApprovalStatus, ModelContentPart, TaskPlan, TaskPlanStep, TaskPlanStepStatus, ToolCall,
    ToolResult,
};
use crate::policy::{PolicyDecision, PolicyEngine, ToolPermissionDescriptor};
use crate::sandbox::LocalSandboxConfig;
use crate::skills::{discover_skills, load_selected_skills};
use crate::spreadsheet::{
    execute_spreadsheet, CellRange, InspectWorkbookRequest, ListSheetsRequest, ReadRangeRequest,
    SheetWriteRequest, SpreadsheetAction, SpreadsheetRequest, SpreadsheetResult,
    WriteWorkbookRequest, MAX_INPUT_FILE_BYTES as MAX_SPREADSHEET_INPUT_BYTES,
};
use crate::store::SessionStore;
use crate::subagents::{SpawnSubagentRequest, SubagentRunStatus, SubagentScheduler, SubagentScope};
use anyhow::Context;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub policy: Arc<dyn PolicyEngine>,
    pub environment: Arc<dyn ExecutionEnvironment>,
    pub store: Option<Arc<dyn SessionStore>>,
    pub thread_id: Option<Uuid>,
    pub cancel: Option<CancellationToken>,
    pub subagents: Option<SubagentScheduler>,
    pub parent_turn_id: Option<Uuid>,
    pub subagent_depth: u8,
    pub browser: Option<Arc<dyn BrowserRuntime>>,
    /// Set only while replaying a tool call that the user explicitly approved.
    /// Browser navigation uses this as a one-time fallback when a caller does not have a
    /// persistent session store from which it can read the approved domain.
    pub approval_granted: bool,
}

impl ToolContext {
    pub fn local(workspace_root: PathBuf, policy: Arc<dyn PolicyEngine>) -> Self {
        Self::local_with_sandbox_config(workspace_root, policy, LocalSandboxConfig::from_env())
    }

    pub fn local_with_sandbox_config(
        workspace_root: PathBuf,
        policy: Arc<dyn PolicyEngine>,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        let environment = Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            workspace_root.clone(),
            sandbox_config,
        ));
        Self {
            workspace_root,
            policy,
            environment,
            store: None,
            thread_id: None,
            cancel: None,
            subagents: None,
            parent_turn_id: None,
            subagent_depth: 0,
            browser: None,
            approval_granted: false,
        }
    }

    pub fn with_environment(
        workspace_root: PathBuf,
        policy: Arc<dyn PolicyEngine>,
        environment: Arc<dyn ExecutionEnvironment>,
    ) -> Self {
        Self {
            workspace_root,
            policy,
            environment,
            store: None,
            thread_id: None,
            cancel: None,
            subagents: None,
            parent_turn_id: None,
            subagent_depth: 0,
            browser: None,
            approval_granted: false,
        }
    }

    fn execution_context(&self, timeout: Duration) -> ExecutionContext {
        let context = ExecutionContext::with_timeout(timeout);
        match &self.cancel {
            Some(cancel) => context.with_cancel(cancel.clone()),
            None => context,
        }
    }
}

fn enforce_policy_decision(decision: PolicyDecision, approval_granted: bool) -> anyhow::Result<()> {
    match decision {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
        PolicyDecision::Ask { .. } if approval_granted => Ok(()),
        PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult>;
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Arc<BTreeMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    pub fn with_builtins() -> Self {
        let mut tools: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
        tools.insert("list_files".to_string(), Arc::new(ListFilesTool));
        tools.insert("read_file".to_string(), Arc::new(ReadFileTool));
        tools.insert("write_file".to_string(), Arc::new(WriteFileTool));
        tools.insert("search".to_string(), Arc::new(SearchTool));
        tools.insert("shell".to_string(), Arc::new(ShellTool));
        tools.insert("git_diff".to_string(), Arc::new(GitDiffTool));
        tools.insert("apply_patch".to_string(), Arc::new(ApplyPatchTool));
        tools.insert("spawn_agent".to_string(), Arc::new(SpawnAgentTool));
        tools.insert("send_input".to_string(), Arc::new(SendAgentInputTool));
        tools.insert("cancel_agent".to_string(), Arc::new(CancelAgentTool));
        tools.insert("wait_agent".to_string(), Arc::new(WaitAgentTool));
        tools.insert("wait_agents".to_string(), Arc::new(WaitAgentsTool));
        tools.insert("update_plan".to_string(), Arc::new(UpdatePlanTool));
        tools.insert("complete_task".to_string(), Arc::new(CompleteTaskTool));
        tools.insert("list_skills".to_string(), Arc::new(ListSkillsTool));
        tools.insert("read_skill".to_string(), Arc::new(ReadSkillTool));
        tools.insert("browser".to_string(), Arc::new(BrowserTool));
        tools.insert("spreadsheet".to_string(), Arc::new(SpreadsheetTool));
        Self {
            tools: Arc::new(tools),
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn insert(&mut self, name: String, tool: Arc<dyn Tool>) {
        let tools = Arc::make_mut(&mut self.tools);
        tools.insert(name, tool);
    }

    pub fn list(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SpreadsheetToolAction {
    Inspect,
    ListSheets,
    ReadRange,
    Write,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpreadsheetToolInput {
    action: SpreadsheetToolAction,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    sheet: Option<String>,
    #[serde(default)]
    range: Option<CellRange>,
    #[serde(default)]
    source_path: Option<String>,
    #[serde(default)]
    output_path: Option<String>,
    #[serde(default)]
    sheets: Vec<SheetWriteRequest>,
}

pub struct SpreadsheetTool;

#[async_trait]
impl Tool for SpreadsheetTool {
    fn name(&self) -> &str {
        "spreadsheet"
    }

    fn description(&self) -> &str {
        "Inspect, list, read, create, or update bounded XLSX workbooks. Uses zero-based row and column coordinates; writes preserve values, formulas, sheet order, and visibility but not formatting or embedded workbook objects."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["inspect", "list_sheets", "read_range", "write"]
                },
                "path": { "type": "string", "description": "Workspace-relative XLSX path for inspect/list/read." },
                "sheet": { "type": "string", "description": "Worksheet name for read_range." },
                "range": {
                    "type": "object",
                    "description": "Inclusive zero-based range for read_range.",
                    "properties": {
                        "start": { "$ref": "#/$defs/address" },
                        "end": { "$ref": "#/$defs/address" }
                    },
                    "required": ["start", "end"],
                    "additionalProperties": false
                },
                "sourcePath": { "type": "string", "description": "Optional existing XLSX to rebuild before applying writes." },
                "outputPath": { "type": "string", "description": "Workspace-relative XLSX output path for write." },
                "sheets": {
                    "type": "array",
                    "maxItems": 256,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "visibility": { "type": "string", "enum": ["visible", "hidden", "very_hidden"] },
                            "cells": {
                                "type": "array",
                                "maxItems": 10000,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "address": { "$ref": "#/$defs/address" },
                                        "value": {
                                            "type": "object",
                                            "properties": {
                                                "type": { "type": "string", "enum": ["blank", "string", "integer", "number", "boolean", "formula"] },
                                                "value": {}
                                            },
                                            "required": ["type"],
                                            "additionalProperties": false
                                        }
                                    },
                                    "required": ["address", "value"],
                                    "additionalProperties": false
                                }
                            }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["action"],
            "additionalProperties": false,
            "$defs": {
                "address": {
                    "type": "object",
                    "properties": {
                        "row": { "type": "integer", "minimum": 0, "maximum": 1048575 },
                        "column": { "type": "integer", "minimum": 0, "maximum": 16383 }
                    },
                    "required": ["row", "column"],
                    "additionalProperties": false
                }
            }
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let input: SpreadsheetToolInput = serde_json::from_value(call.input.clone())
            .context("spreadsheet received invalid arguments")?;
        match input.action {
            SpreadsheetToolAction::Inspect
            | SpreadsheetToolAction::ListSheets
            | SpreadsheetToolAction::ReadRange => {
                execute_spreadsheet_read(call.id, input, ctx).await
            }
            SpreadsheetToolAction::Write => execute_spreadsheet_write(call.id, input, ctx).await,
        }
    }
}

async fn execute_spreadsheet_read(
    call_id: Uuid,
    input: SpreadsheetToolInput,
    ctx: ToolContext,
) -> anyhow::Result<ToolResult> {
    let relative = input
        .path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("spreadsheet read action requires path")?;
    let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
    enforce_read_policy(&ctx, &logical_path)?;
    let resolved_path = ctx.environment.resolve_read_path(&logical_path)?;
    ensure_xlsx_path(&resolved_path)?;
    let read = ctx
        .environment
        .read_file(FileReadRequest::new(&resolved_path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
        .await?;
    let resolved_path = read.path.clone();
    let source_path = resolved_path.clone();
    let source_bytes = read.bytes;
    let action = input.action;
    let sheet = input.sheet;
    let range = input.range;
    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_input = staging.path("input.xlsx");
        fs::write(&staged_input, source_bytes)
            .with_context(|| format!("failed to stage {}", source_path.display()))?;
        let action = match action {
            SpreadsheetToolAction::Inspect => {
                SpreadsheetAction::InspectWorkbook(InspectWorkbookRequest { path: staged_input })
            }
            SpreadsheetToolAction::ListSheets => {
                SpreadsheetAction::ListSheets(ListSheetsRequest { path: staged_input })
            }
            SpreadsheetToolAction::ReadRange => SpreadsheetAction::ReadRange(ReadRangeRequest {
                path: staged_input,
                sheet: sheet
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .context("spreadsheet read_range requires sheet")?,
                range: range.context("spreadsheet read_range requires range")?,
            }),
            SpreadsheetToolAction::Write => unreachable!(),
        };
        Ok(execute_spreadsheet(SpreadsheetRequest { action }))
    })
    .await
    .context("spreadsheet worker task failed")??;
    let mut result = match outcome {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    remap_spreadsheet_paths(&mut result, Some(&resolved_path), None);
    spreadsheet_success_result(call_id, result, None)
}

async fn execute_spreadsheet_write(
    call_id: Uuid,
    input: SpreadsheetToolInput,
    ctx: ToolContext,
) -> anyhow::Result<ToolResult> {
    let output_relative = input
        .output_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .context("spreadsheet write requires outputPath")?;
    let output_path = normalize_workspace_path(&ctx.workspace_root, output_relative)?;
    ensure_xlsx_path(&output_path)?;
    enforce_policy_decision(ctx.policy.inspect_write(&output_path), ctx.approval_granted)?;

    let source = if let Some(relative) = input
        .source_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;
        ensure_xlsx_path(&path)?;
        Some(
            ctx.environment
                .read_file(FileReadRequest::new(&path).with_max_bytes(MAX_SPREADSHEET_INPUT_BYTES))
                .await?,
        )
    } else {
        None
    };

    let sheets = input.sheets;
    let staged = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let staging = SpreadsheetStaging::new()?;
        let staged_source = if let Some(source) = source {
            let path = staging.path("source.xlsx");
            fs::write(&path, source.bytes)
                .with_context(|| format!("failed to stage {}", source.path.display()))?;
            Some(path)
        } else {
            None
        };
        let staged_output = staging.path("output.xlsx");
        let outcome = execute_spreadsheet(SpreadsheetRequest {
            action: SpreadsheetAction::WriteWorkbook(WriteWorkbookRequest {
                source: staged_source,
                output: staged_output.clone(),
                sheets,
            }),
        });
        match outcome {
            Ok(result) => {
                let bytes = fs::read(&staged_output)
                    .with_context(|| format!("failed to read {}", staged_output.display()))?;
                Ok(Ok((result, bytes)))
            }
            Err(error) => Ok(Err(error)),
        }
    })
    .await
    .context("spreadsheet worker task failed")??;
    let (mut result, bytes) = match staged {
        Ok(result) => result,
        Err(error) => return Ok(spreadsheet_error_result(call_id, error)),
    };
    let written = ctx
        .environment
        .write_file(FileWriteRequest::new(&output_path, bytes))
        .await?;
    remap_spreadsheet_paths(&mut result, None, Some(&written.path));
    spreadsheet_success_result(call_id, result, Some(written.path))
}

fn spreadsheet_success_result(
    call_id: Uuid,
    result: SpreadsheetResult,
    changed_path: Option<PathBuf>,
) -> anyhow::Result<ToolResult> {
    let action = result.kind();
    let value = serde_json::to_value(&result)?;
    let output = serde_json::to_string_pretty(&value)?;
    let mut content = vec![ModelContentPart::json(value.clone())];
    let mut metadata = json!({
        "toolName": "spreadsheet",
        "action": action,
        "success": true
    });
    if let Some(path) = changed_path {
        content.push(ModelContentPart::resource(
            path.to_string_lossy(),
            Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()),
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
        ));
        if let Some(object) = metadata.as_object_mut() {
            object.insert("changedPath".to_string(), json!(path));
        }
    }
    Ok(ToolResult {
        call_id,
        output,
        content,
        metadata,
    })
}

fn spreadsheet_error_result(
    call_id: Uuid,
    error: crate::spreadsheet::SpreadsheetError,
) -> ToolResult {
    let info = error.info();
    ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&info).unwrap_or_else(|_| error.to_string()),
        content: vec![ModelContentPart::json(
            serde_json::to_value(&info).unwrap_or_else(|_| json!({ "message": error.to_string() })),
        )],
        metadata: json!({
            "toolName": "spreadsheet",
            "success": false,
            "errorCode": info.code,
            "error": info.message
        }),
    }
}

fn remap_spreadsheet_paths(
    result: &mut SpreadsheetResult,
    source: Option<&Path>,
    output: Option<&Path>,
) {
    match result {
        SpreadsheetResult::WorkbookInspected(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::SheetsListed(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::RangeRead(result) => {
            if let Some(source) = source {
                result.path = source.to_path_buf();
            }
        }
        SpreadsheetResult::WorkbookWritten(result) => {
            if let Some(output) = output {
                result.output = output.to_path_buf();
            }
        }
    }
}

fn ensure_xlsx_path(path: &Path) -> anyhow::Result<()> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xlsx"))
    {
        Ok(())
    } else {
        anyhow::bail!("spreadsheet tool supports only .xlsx files")
    }
}

struct SpreadsheetStaging {
    root: PathBuf,
}

impl SpreadsheetStaging {
    fn new() -> anyhow::Result<Self> {
        let root = std::env::temp_dir().join(format!("opentopia-xlsx-{}", Uuid::new_v4()));
        fs::create_dir_all(&root)
            .with_context(|| format!("failed to create {}", root.display()))?;
        Ok(Self { root })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for SpreadsheetStaging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const MAX_TASK_COMPLETION_SUMMARY_CHARS: usize = 4_000;
const MAX_TASK_COMPLETION_ITEMS: usize = 20;
const MAX_TASK_COMPLETION_ITEM_CHARS: usize = 1_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteTaskInput {
    summary: String,
    #[serde(default)]
    verification: Vec<String>,
    #[serde(default)]
    remaining_work: Vec<String>,
}

pub struct CompleteTaskTool;

#[async_trait]
impl Tool for CompleteTaskTool {
    fn name(&self) -> &str {
        "complete_task"
    }

    fn description(&self) -> &str {
        "Finish the current user task after its requested scope has been verified. Provide a concise summary, concrete verification evidence, and any deliberately deferred work. This is the final tool call for the turn."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Concise description of the completed result."
                },
                "verification": {
                    "type": "array",
                    "maxItems": MAX_TASK_COMPLETION_ITEMS,
                    "items": { "type": "string" },
                    "description": "Commands, checks, or observed results that verify the completed scope."
                },
                "remaining_work": {
                    "type": "array",
                    "maxItems": MAX_TASK_COMPLETION_ITEMS,
                    "items": { "type": "string" },
                    "description": "Work intentionally left for a later phase. Empty means no known remaining work."
                }
            },
            "required": ["summary", "verification", "remaining_work"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, _ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let input: CompleteTaskInput = serde_json::from_value(call.input.clone())
            .context("complete_task received invalid arguments")?;
        let summary =
            validate_completion_text("summary", input.summary, MAX_TASK_COMPLETION_SUMMARY_CHARS)?;
        let verification = validate_completion_items("verification", input.verification)?;
        let remaining_work = validate_completion_items("remaining_work", input.remaining_work)?;

        let mut output = summary.clone();
        if !verification.is_empty() {
            output.push_str("\n\nVerification:\n");
            for item in &verification {
                output.push_str("- ");
                output.push_str(item);
                output.push('\n');
            }
            output.pop();
        }
        if !remaining_work.is_empty() {
            output.push_str("\n\nRemaining work:\n");
            for item in &remaining_work {
                output.push_str("- ");
                output.push_str(item);
                output.push('\n');
            }
            output.pop();
        }

        let completion = json!({
            "summary": summary,
            "verification": verification,
            "remainingWork": remaining_work
        });
        Ok(ToolResult {
            call_id: call.id,
            output,
            content: vec![ModelContentPart::json(completion.clone())],
            metadata: json!({
                "toolName": self.name(),
                "taskCompletion": completion,
                "success": true
            }),
        })
    }
}

fn validate_completion_text(
    field: &str,
    value: String,
    max_chars: usize,
) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("complete_task {field} cannot be empty");
    }
    if value.chars().count() > max_chars {
        anyhow::bail!("complete_task {field} exceeds the {max_chars} character limit");
    }
    Ok(value)
}

fn validate_completion_items(field: &str, values: Vec<String>) -> anyhow::Result<Vec<String>> {
    if values.len() > MAX_TASK_COMPLETION_ITEMS {
        anyhow::bail!(
            "complete_task {field} may contain at most {MAX_TASK_COMPLETION_ITEMS} items"
        );
    }
    values
        .into_iter()
        .map(|value| validate_completion_text(field, value, MAX_TASK_COMPLETION_ITEM_CHARS))
        .collect()
}

const MAX_TASK_PLAN_STEPS: usize = 20;
const MAX_TASK_PLAN_STEP_CHARS: usize = 300;
const MAX_TASK_PLAN_EXPLANATION_CHARS: usize = 2_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlanInput {
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    current_scope_complete: bool,
    #[serde(default)]
    verification: Vec<String>,
    plan: Vec<TaskPlanStep>,
}

pub struct UpdatePlanTool;

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Replace the current task checklist with concise steps and their progress. Use it for multi-step work and keep it current. When the current requested scope is verified but later-phase steps deliberately remain pending, set current_scope_complete and include verification evidence."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "explanation": {
                    "type": "string",
                    "description": "Optional reason for changing the plan."
                },
                "current_scope_complete": {
                    "type": "boolean",
                    "description": "True only when every step in the current user-requested scope is complete and verified; explicitly deferred later-phase steps may remain pending."
                },
                "verification": {
                    "type": "array",
                    "maxItems": MAX_TASK_COMPLETION_ITEMS,
                    "items": { "type": "string" },
                    "description": "Concrete checks supporting current_scope_complete, such as a successful test command."
                },
                "plan": {
                    "type": "array",
                    "maxItems": MAX_TASK_PLAN_STEPS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": { "type": "string", "description": "Short, verifiable task step." },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"]
                            }
                        },
                        "required": ["step", "status"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["plan"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        if ctx.subagent_depth > 0 {
            anyhow::bail!("only the parent agent may update the shared task plan");
        }
        let input: UpdatePlanInput = serde_json::from_value(call.input.clone())
            .context("update_plan received invalid arguments")?;
        if input.plan.len() > MAX_TASK_PLAN_STEPS {
            anyhow::bail!("task plan may contain at most {MAX_TASK_PLAN_STEPS} steps");
        }

        let mut in_progress = 0usize;
        let mut unique_steps = HashSet::new();
        let mut steps = Vec::with_capacity(input.plan.len());
        for item in input.plan {
            let step = item.step.trim();
            if step.is_empty() {
                anyhow::bail!("task plan steps cannot be empty");
            }
            if step.chars().count() > MAX_TASK_PLAN_STEP_CHARS {
                anyhow::bail!(
                    "task plan step exceeds the {MAX_TASK_PLAN_STEP_CHARS} character limit"
                );
            }
            let normalized = step.to_lowercase();
            if !unique_steps.insert(normalized) {
                anyhow::bail!("task plan contains duplicate step: {step}");
            }
            if item.status == TaskPlanStepStatus::InProgress {
                in_progress += 1;
            }
            steps.push(TaskPlanStep {
                step: step.to_string(),
                status: item.status,
            });
        }
        if in_progress > 1 {
            anyhow::bail!("task plan may contain at most one in_progress step");
        }
        if input.current_scope_complete && in_progress > 0 {
            anyhow::bail!("a completed current scope cannot contain an in_progress step");
        }

        let explanation = input
            .explanation
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if explanation
            .as_ref()
            .is_some_and(|value| value.chars().count() > MAX_TASK_PLAN_EXPLANATION_CHARS)
        {
            anyhow::bail!(
                "task plan explanation exceeds the {MAX_TASK_PLAN_EXPLANATION_CHARS} character limit"
            );
        }

        let plan = TaskPlan { explanation, steps };
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == TaskPlanStepStatus::Completed)
            .count();
        let verification = validate_completion_items("verification", input.verification)?;
        if input.current_scope_complete && completed == 0 {
            anyhow::bail!("a completed current scope must contain a completed plan step");
        }
        if input.current_scope_complete && verification.is_empty() {
            anyhow::bail!("a completed current scope requires verification evidence");
        }
        let value = serde_json::to_value(&plan)?;
        Ok(ToolResult {
            call_id: call.id,
            output: format!(
                "Plan updated: {completed}/{} steps completed.",
                plan.steps.len()
            ),
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": self.name(),
                "taskPlan": value,
                "completed": completed,
                "total": plan.steps.len(),
                "allStepsComplete": !plan.steps.is_empty() && completed == plan.steps.len(),
                "currentScopeComplete": input.current_scope_complete,
                "verification": verification,
                "success": true
            }),
        })
    }
}

pub struct ListSkillsTool;

#[async_trait]
impl Tool for ListSkillsTool {
    fn name(&self) -> &str {
        "list_skills"
    }

    fn description(&self) -> &str {
        "List available capability instructions (Skills) without loading their instructions."
    }

    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let skills = discover_skills(Some(&ctx.workspace_root));
        let value = serde_json::to_value(&skills)?;
        Ok(ToolResult {
            call_id: call.id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({ "count": skills.len() }),
        })
    }
}

pub struct ReadSkillTool;

#[async_trait]
impl Tool for ReadSkillTool {
    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Read one Skill's instructions after deciding it is relevant to the current task."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Skill ID returned by list_skills." } },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let id = required_string(&call.input, "id")?;
        // load_selected_skills resolves the opaque ID against the bounded, canonicalized Skill
        // catalog. It cannot be used as a general-purpose path read, including for user Skills
        // that intentionally live outside the thread workspace.
        let loaded = load_selected_skills(Some(&ctx.workspace_root), &[id])?
            .into_iter()
            .next()
            .context("Skill is unavailable")?;
        let output = loaded.render_for_model();
        Ok(ToolResult {
            call_id: call.id,
            output: output.clone(),
            content: vec![ModelContentPart::text(output)],
            metadata: json!({
                "id": loaded.descriptor.id,
                "name": loaded.descriptor.name,
                "path": loaded.descriptor.path,
                "truncated": loaded.truncated
            }),
        })
    }
}

pub struct BrowserTool;

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Use an isolated local browser to navigate, inspect pages, take screenshots, click, type, wait, download, or close the current thread's browser session. The first visit to each domain requires user approval."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "snapshot", "screenshot", "click", "type", "wait", "download", "close"],
                    "description": "Browser action to perform."
                },
                "url": { "type": "string", "description": "URL for navigate or download." },
                "selector": { "type": "string", "description": "CSS selector for click, type, or wait." },
                "text": { "type": "string", "description": "Text for type or a wait text condition." },
                "clearFirst": { "type": "boolean", "description": "Clear an input before typing; defaults to true." },
                "condition": {
                    "type": "string",
                    "enum": ["document_complete", "selector", "text"],
                    "description": "Wait condition; defaults to document_complete."
                },
                "timeoutMs": { "type": "integer", "minimum": 1, "maximum": 120000 },
                "expectedFilename": { "type": "string", "description": "Optional expected filename for a download." }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let runtime = ctx
            .browser
            .as_ref()
            .context("browser runtime is unavailable")?
            .clone();
        let thread_id = ctx.thread_id.context("browser requires a thread context")?;
        let session = BrowserSessionId::from_thread(thread_id);
        let action = required_string(&call.input, "action")?;
        let timeout = browser_timeout(&call.input);
        let output = match action.as_str() {
            "navigate" => {
                let url = required_string(&call.input, "url")?;
                inspect_browser_url(&ctx, &url)?;
                let mut request = BrowserNavigateRequest::new(url);
                if let Some(wait) = request.wait.as_mut() {
                    wait.timeout = timeout;
                }
                runtime.navigate(session, request).await?
            }
            "snapshot" => runtime.snapshot(session).await?,
            "screenshot" => runtime.screenshot(session).await?,
            "click" => {
                inspect_browser_interaction(&ctx)?;
                runtime
                    .click(
                        session,
                        BrowserSelector::new(required_string(&call.input, "selector")?)?,
                    )
                    .await?
            }
            "type" => {
                inspect_browser_interaction(&ctx)?;
                runtime
                    .type_text(
                        session,
                        BrowserTypeRequest {
                            selector: BrowserSelector::new(required_string(
                                &call.input,
                                "selector",
                            )?)?,
                            text: required_string(&call.input, "text")?,
                            clear_first: call
                                .input
                                .get("clearFirst")
                                .and_then(Value::as_bool)
                                .unwrap_or(true),
                        },
                    )
                    .await?
            }
            "wait" => {
                let condition = match call
                    .input
                    .get("condition")
                    .and_then(Value::as_str)
                    .unwrap_or("document_complete")
                {
                    "document_complete" => BrowserWaitCondition::DocumentComplete,
                    "selector" => BrowserWaitCondition::Selector(BrowserSelector::new(
                        required_string(&call.input, "selector")?,
                    )?),
                    "text" => BrowserWaitCondition::Text(required_string(&call.input, "text")?),
                    other => anyhow::bail!("unsupported browser wait condition: {other}"),
                };
                runtime
                    .wait(
                        session,
                        BrowserWaitRequest {
                            condition,
                            timeout,
                            poll_interval: Duration::from_millis(100),
                        },
                    )
                    .await?
            }
            "download" => {
                let url = required_string(&call.input, "url")?;
                inspect_browser_url(&ctx, &url)?;
                runtime
                    .download(
                        session,
                        BrowserDownloadRequest {
                            url,
                            expected_filename: call
                                .input
                                .get("expectedFilename")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            timeout,
                        },
                    )
                    .await?
            }
            "close" => {
                inspect_browser_interaction(&ctx)?;
                runtime.close_session(session).await?;
                return Ok(ToolResult::text(
                    call.id,
                    "Closed the isolated browser session for this thread.",
                    json!({ "sessionId": session.to_string(), "action": action }),
                ));
            }
            other => anyhow::bail!("unsupported browser action: {other}"),
        };
        Ok(browser_output_to_tool_result(call.id, action, output))
    }
}

fn browser_timeout(input: &Value) -> Option<Duration> {
    input
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .map(|milliseconds| Duration::from_millis(milliseconds.clamp(1, 120_000)))
}

const BROWSER_DOMAIN_APPROVAL_PREFIX: &str = "browser:domain:";

/// Parse a browser URL into the normalized host used for policy checks and persisted approvals.
pub fn browser_domain_from_url(raw_url: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(raw_url).context("browser URL is invalid")?;
    let host = url.host_str().context("browser URL must include a host")?;
    Ok(host.trim_end_matches('.').to_ascii_lowercase())
}

/// Keep domain grants in the existing approval history so they survive runtime restarts and are
/// scoped by the approval's thread ID.
pub fn browser_domain_approval_action(host: &str) -> String {
    format!(
        "{BROWSER_DOMAIN_APPROVAL_PREFIX}{}",
        host.trim_end_matches('.').to_ascii_lowercase()
    )
}

pub fn browser_domain_from_approval_action(action: &str) -> Option<String> {
    action
        .strip_prefix(BROWSER_DOMAIN_APPROVAL_PREFIX)
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
}

pub fn browser_domain_is_approved(
    store: &dyn SessionStore,
    thread_id: Uuid,
    host: &str,
) -> anyhow::Result<bool> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    Ok(store
        .list_approvals(thread_id, Some(ApprovalStatus::Approved))?
        .into_iter()
        .filter_map(|approval| browser_domain_from_approval_action(&approval.action))
        .any(|approved_host| approved_host == host))
}

fn inspect_browser_url(ctx: &ToolContext, raw_url: &str) -> anyhow::Result<()> {
    let host = browser_domain_from_url(raw_url)?;
    enforce_policy_decision(ctx.policy.inspect_network(&host), ctx.approval_granted)?;

    if ctx.approval_granted
        || ctx
            .store
            .as_deref()
            .map(|store| {
                browser_domain_is_approved(store, ctx.thread_id.unwrap_or_default(), &host)
            })
            .transpose()?
            .unwrap_or(false)
    {
        return Ok(());
    }

    anyhow::bail!("approval required: Browser access to the new domain `{host}` requires approval.")
}

fn inspect_browser_interaction(ctx: &ToolContext) -> anyhow::Result<()> {
    enforce_policy_decision(
        ctx.policy.inspect_network("browser-interaction"),
        ctx.approval_granted,
    )
}

fn browser_output_to_tool_result(
    call_id: Uuid,
    action: String,
    output: crate::browser::BrowserOutput,
) -> ToolResult {
    let mut rendered = Vec::new();
    let mut content = Vec::new();
    for item in output.contents {
        match item {
            BrowserContent::Text { text, truncated } => {
                if truncated {
                    rendered.push(format!("{text}\n\n[Browser text truncated]"));
                } else {
                    rendered.push(text.clone());
                }
                content.push(ModelContentPart::text(text));
            }
            BrowserContent::Json { value } => {
                rendered.push(value.to_string());
                content.push(ModelContentPart::json(value));
            }
            BrowserContent::Image { mime_type, bytes } => {
                rendered.push(format!("[Browser screenshot: {} bytes]", bytes.len()));
                content.push(ModelContentPart::image(mime_type, bytes));
            }
            BrowserContent::File {
                path,
                mime_type,
                bytes,
            } => {
                rendered.push(format!(
                    "[Browser download: {} ({} bytes)]",
                    path.display(),
                    bytes
                ));
                content.push(ModelContentPart::resource(
                    path.to_string_lossy(),
                    mime_type,
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(str::to_string),
                ));
            }
        }
    }
    ToolResult {
        call_id,
        output: rendered.join("\n\n"),
        content,
        metadata: json!({ "action": action, "url": output.url, "browser": output.metadata }),
    }
}

pub struct SpawnAgentTool;

#[async_trait]
impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Start a bounded child agent that inherits the current workspace, provider, permissions, and sandbox."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short worker name." },
                "input": { "type": "string", "description": "Concrete task for the child agent." }
            },
            "required": ["name", "input"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let thread_id = ctx
            .thread_id
            .context("subagent parent thread is unavailable")?;
        let parent_turn_id = ctx
            .parent_turn_id
            .context("subagent parent turn is unavailable")?;
        let name = required_string(&call.input, "name")?;
        let input = required_string(&call.input, "input")?;
        let run = scheduler.spawn(SpawnSubagentRequest {
            parent_thread_id: thread_id,
            parent_turn_id,
            name,
            input,
            depth: ctx.subagent_depth.saturating_add(1),
        })?;
        Ok(ToolResult {
            call_id: call.id,
            output: serde_json::to_string(&run)?,
            content: Vec::new(),
            metadata: json!({
                "toolName": self.name(),
                "runId": run.id,
                "status": run.status,
                "success": true
            }),
        })
    }
}

pub struct SendAgentInputTool;

#[async_trait]
impl Tool for SendAgentInputTool {
    fn name(&self) -> &str {
        "send_input"
    }

    fn description(&self) -> &str {
        "Send additional input to an active child agent."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "runId": { "type": "string", "description": "Child run UUID." },
                "input": { "type": "string", "description": "Additional instructions." }
            },
            "required": ["runId", "input"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let run_id = required_uuid(&call.input, "runId")?;
        scheduler.send_input_scoped(
            subagent_scope(&ctx)?,
            run_id,
            required_string(&call.input, "input")?,
        )?;
        Ok(ToolResult {
            call_id: call.id,
            output: format!("Input delivered to subagent {run_id}."),
            content: Vec::new(),
            metadata: json!({ "toolName": self.name(), "runId": run_id, "success": true }),
        })
    }
}

pub struct CancelAgentTool;

#[async_trait]
impl Tool for CancelAgentTool {
    fn name(&self) -> &str {
        "cancel_agent"
    }

    fn description(&self) -> &str {
        "Cancel an active child agent."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "runId": { "type": "string", "description": "Child run UUID." } },
            "required": ["runId"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let run_id = required_uuid(&call.input, "runId")?;
        scheduler.cancel_scoped(subagent_scope(&ctx)?, run_id)?;
        Ok(ToolResult {
            call_id: call.id,
            output: format!("Cancellation requested for subagent {run_id}."),
            content: Vec::new(),
            metadata: json!({ "toolName": self.name(), "runId": run_id, "success": true }),
        })
    }
}

pub struct WaitAgentTool;

#[async_trait]
impl Tool for WaitAgentTool {
    fn name(&self) -> &str {
        "wait_agent"
    }

    fn description(&self) -> &str {
        "Wait for a child agent to finish and return its persisted status and result."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "runId": { "type": "string", "description": "Child run UUID." },
                "timeoutMs": { "type": "integer", "minimum": 1, "maximum": 120000 }
            },
            "required": ["runId"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let run_id = required_uuid(&call.input, "runId")?;
        let timeout_ms = call
            .input
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(30_000)
            .clamp(1, 120_000);
        let run = scheduler
            .wait_scoped(
                subagent_scope(&ctx)?,
                run_id,
                Duration::from_millis(timeout_ms),
            )
            .await?;
        Ok(ToolResult {
            call_id: call.id,
            output: serde_json::to_string(&run)?,
            content: Vec::new(),
            metadata: json!({
                "toolName": self.name(),
                "runId": run_id,
                "status": run.status,
                "terminal": run.status.is_terminal(),
                "success": run.status == SubagentRunStatus::Completed
            }),
        })
    }
}

const MAX_BATCH_WAIT_AGENTS: usize = 8;

pub struct WaitAgentsTool;

#[async_trait]
impl Tool for WaitAgentsTool {
    fn name(&self) -> &str {
        "wait_agents"
    }

    fn description(&self) -> &str {
        "Wait concurrently for multiple direct child agents and return every completed result or timeout error in one structured response."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "runIds": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_BATCH_WAIT_AGENTS,
                    "items": { "type": "string", "description": "Child run UUID." }
                },
                "timeoutMs": { "type": "integer", "minimum": 1, "maximum": 120000 }
            },
            "required": ["runIds"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let raw_ids = call
            .input
            .get("runIds")
            .and_then(Value::as_array)
            .context("wait_agents requires runIds")?;
        if raw_ids.is_empty() || raw_ids.len() > MAX_BATCH_WAIT_AGENTS {
            anyhow::bail!("wait_agents requires between 1 and {MAX_BATCH_WAIT_AGENTS} run IDs");
        }
        let mut unique = HashSet::new();
        let mut run_ids = Vec::with_capacity(raw_ids.len());
        for value in raw_ids {
            let raw = value
                .as_str()
                .context("wait_agents runIds must contain UUID strings")?;
            let run_id = Uuid::parse_str(raw).context("wait_agents received an invalid run ID")?;
            if !unique.insert(run_id) {
                anyhow::bail!("wait_agents received duplicate run ID {run_id}");
            }
            run_ids.push(run_id);
        }

        let timeout_ms = call
            .input
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(30_000)
            .clamp(1, 120_000);
        let timeout = Duration::from_millis(timeout_ms);
        let scope = subagent_scope(&ctx)?;
        let waits = run_ids
            .iter()
            .map(|run_id| scheduler.wait_scoped(scope, *run_id, timeout));
        let outcomes = futures_util::future::join_all(waits).await;
        let runs = run_ids
            .iter()
            .zip(outcomes)
            .map(|(run_id, outcome)| match outcome {
                Ok(run) => json!({
                    "runId": run_id,
                    "status": run.status,
                    "result": run.result,
                    "error": run.error,
                    "terminal": run.status.is_terminal(),
                    "success": run.status == SubagentRunStatus::Completed
                }),
                Err(error) => json!({
                    "runId": run_id,
                    "terminal": false,
                    "success": false,
                    "waitError": error.to_string()
                }),
            })
            .collect::<Vec<_>>();
        let all_terminal = runs
            .iter()
            .all(|run| run.get("terminal").and_then(Value::as_bool) == Some(true));
        let all_succeeded = runs
            .iter()
            .all(|run| run.get("success").and_then(Value::as_bool) == Some(true));
        let value = json!({
            "runs": runs,
            "allTerminal": all_terminal,
            "allSucceeded": all_succeeded
        });
        Ok(ToolResult {
            call_id: call.id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": self.name(),
                "runCount": run_ids.len(),
                "allTerminal": all_terminal,
                "allSucceeded": all_succeeded,
                "success": all_succeeded
            }),
        })
    }
}

fn subagent_scope(ctx: &ToolContext) -> anyhow::Result<SubagentScope> {
    Ok(SubagentScope {
        thread_id: ctx
            .thread_id
            .context("subagent parent thread is unavailable")?,
        parent_turn_id: ctx
            .parent_turn_id
            .context("subagent parent turn is unavailable")?,
        depth: ctx.subagent_depth,
    })
}

fn required_string(input: &Value, key: &str) -> anyhow::Result<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("{key} must be a non-empty string"))
}

fn required_uuid(input: &Value, key: &str) -> anyhow::Result<Uuid> {
    let value = required_string(input, key)?;
    Uuid::parse_str(&value).with_context(|| format!("{key} must be a UUID"))
}

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "List direct children of a directory inside the workspace."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path relative to workspace." }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let relative = call
            .input
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".");
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let entries = tokio::task::spawn_blocking(move || list_dir_entries(&path))
            .await
            .context("list_files task failed")??;
        Ok(ToolResult {
            call_id: call.id,
            output: entries.join("\n"),
            content: Vec::new(),
            metadata: json!({ "count": entries.len() }),
        })
    }
}

pub struct ReadFileTool;

const READ_FILE_ARTIFACT_THRESHOLD: usize = 64_000;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file inside the workspace."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path relative to workspace." }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let relative = call
            .input
            .get("path")
            .and_then(Value::as_str)
            .context("read_file requires a path")?;
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let read = ctx
            .environment
            .read_file(FileReadRequest::new(&path))
            .await?;
        let contents = String::from_utf8(read.bytes)
            .with_context(|| format!("failed to read {} as UTF-8", read.path.display()))?;
        let bytes = contents.len();
        let mut output = truncate(&contents, 16_000);
        let mut metadata = json!({
            "path": read.path.display().to_string(),
            "bytes": bytes
        });

        if bytes > READ_FILE_ARTIFACT_THRESHOLD {
            if let Some(ref store) = ctx.store {
                if let Some(thread_id) = ctx.thread_id {
                    let tool_result = ToolResult {
                        call_id: call.id,
                        output: contents,
                        content: Vec::new(),
                        metadata: metadata.clone(),
                    };
                    if let Ok(Some(artifact)) = store.insert_large_tool_output_artifact(
                        thread_id,
                        &tool_result,
                        READ_FILE_ARTIFACT_THRESHOLD,
                    ) {
                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert("artifactId".to_string(), json!(artifact.id));
                            obj.insert("artifactKind".to_string(), json!("file_content"));
                            obj.insert(
                                "artifact".to_string(),
                                json!({
                                    "id": artifact.id,
                                    "kind": "file_content",
                                    "bytes": bytes
                                }),
                            );
                        }
                        output.push_str(&format!("\n\n[Artifact: {}]", artifact.id));
                    }
                }
            }
        }

        Ok(ToolResult {
            call_id: call.id,
            output,
            content: Vec::new(),
            metadata,
        })
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write a UTF-8 text file inside the workspace."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path relative to workspace." },
                "content": { "type": "string", "description": "Full file contents to write." }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let relative = call
            .input
            .get("path")
            .and_then(Value::as_str)
            .context("write_file requires a path")?;
        let content = call
            .input
            .get("content")
            .and_then(Value::as_str)
            .context("write_file requires content")?;
        let path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_policy_decision(ctx.policy.inspect_write(&path), ctx.approval_granted)?;

        let written = ctx
            .environment
            .write_file(FileWriteRequest::new(&path, content.as_bytes().to_vec()))
            .await?;
        Ok(ToolResult {
            call_id: call.id,
            output: format!(
                "Wrote {} bytes to {}",
                written.bytes_written,
                written.path.display()
            ),
            content: Vec::new(),
            metadata: json!({
                "changedPath": written.path.display().to_string(),
                "bytes": written.bytes_written
            }),
        })
    }
}

pub struct SearchTool;

const DEFAULT_SEARCH_MAX_RESULTS: usize = 100;
const SEARCH_MAX_RESULTS_LIMIT: usize = 1_000;
const SEARCH_OUTPUT_MAX_BYTES: usize = 32_000;
const SEARCH_ARTIFACT_THRESHOLD: usize = 32_000;
const FALLBACK_MAX_FILE_BYTES: u64 = 1_048_576;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search workspace text with ripgrep, falling back to a simple substring scan."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search pattern passed to rg, or substring for fallback search." },
                "path": { "type": "string", "description": "Optional file or directory path relative to workspace." },
                "maxResults": { "type": "number", "description": "Maximum matching lines to return." }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let query = call
            .input
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .context("search requires a query")?;
        let relative = call
            .input
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(".");
        let max_results = call
            .input
            .get("maxResults")
            .or_else(|| call.input.get("max_results"))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_SEARCH_MAX_RESULTS)
            .min(SEARCH_MAX_RESULTS_LIMIT);

        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let search_arg = search_command_path(relative, &path);
        let result =
            match run_rg_search(ctx.environment.as_ref(), &search_arg, query, max_results).await? {
                Some(result) => result,
                None => {
                    run_fallback_search(
                        ctx.workspace_root.clone(),
                        path.clone(),
                        ctx.policy.clone(),
                        query.to_string(),
                        max_results,
                    )
                    .await?
                }
            };

        let metadata = json!({
            "query": query,
            "path": path.display().to_string(),
            "engine": result.engine,
            "matches": result.matches,
            "returnedMatches": result.returned_matches,
            "maxResults": max_results,
            "truncated": result.truncated,
            "originalBytes": result.original_bytes,
            "outputBytes": result.output_bytes,
            "fallback": result.fallback
        });

        let mut tool_result = ToolResult {
            call_id: call.id,
            output: result.output,
            content: Vec::new(),
            metadata,
        };

        if let Some(ref store) = ctx.store {
            if let Some(thread_id) = ctx.thread_id {
                if tool_result.output.len() > SEARCH_ARTIFACT_THRESHOLD {
                    if let Ok(Some(artifact)) = store.insert_large_tool_output_artifact(
                        thread_id,
                        &tool_result,
                        SEARCH_ARTIFACT_THRESHOLD,
                    ) {
                        if let Some(obj) = tool_result.metadata.as_object_mut() {
                            obj.insert("artifactId".to_string(), json!(artifact.id));
                            obj.insert("artifactKind".to_string(), json!("tool_output"));
                            obj.insert(
                                "artifact".to_string(),
                                json!({
                                    "id": artifact.id,
                                    "kind": "tool_output",
                                    "bytes": tool_result.output.len()
                                }),
                            );
                        }
                        tool_result
                            .output
                            .push_str(&format!("\n\n[Artifact: {}]", artifact.id));
                    }
                } else if let Some(obj) = tool_result.metadata.as_object_mut() {
                    obj.insert(
                        "artifact".to_string(),
                        json!({
                            "kind": "tool_output",
                            "contentType": "text/plain",
                            "status": "inline",
                            "eligible": result.truncated
                        }),
                    );
                }
            }
        }

        Ok(tool_result)
    }
}

pub struct ShellTool;

const ARTIFACT_THRESHOLD: usize = 16_000;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Run a shell command in the workspace with timeout and output caps."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Command to run." },
                "timeoutSeconds": { "type": "number", "description": "Timeout in seconds." }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let command = call
            .input
            .get("command")
            .and_then(Value::as_str)
            .context("shell requires a command")?;
        enforce_policy_decision(ctx.policy.inspect_command(command), ctx.approval_granted)?;

        let timeout_seconds = call
            .input
            .get("timeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .min(300);

        let output = ctx
            .environment
            .exec(
                ExecRequest::shell(command).cwd(&ctx.workspace_root),
                ctx.execution_context(Duration::from_secs(timeout_seconds)),
            )
            .await?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.success && looks_like_sandbox_denial(&stderr) {
            anyhow::bail!(
                "approval required: command was blocked by the sandbox: {}",
                truncate(&stderr, 2_000)
            );
        }
        let full_combined = format!(
            "$ {}\n\n[stdout]\n{}\n\n[stderr]\n{}",
            command, stdout, stderr
        );
        let combined = format!(
            "$ {}\n\n[stdout]\n{}\n\n[stderr]\n{}",
            command,
            truncate(&stdout, 24_000),
            truncate(&stderr, 12_000)
        );

        let mut result = ToolResult {
            call_id: call.id,
            output: combined,
            content: Vec::new(),
            metadata: json!({
                "exitCode": output.exit_code,
                "success": output.success,
                "sandbox": output.sandbox
            }),
        };

        if let Some(ref store) = ctx.store {
            if let Some(thread_id) = ctx.thread_id {
                if full_combined.len() > ARTIFACT_THRESHOLD {
                    let artifact_result = ToolResult {
                        call_id: result.call_id,
                        output: full_combined,
                        content: Vec::new(),
                        metadata: result.metadata.clone(),
                    };
                    if let Ok(Some(artifact)) = store.insert_large_tool_output_artifact(
                        thread_id,
                        &artifact_result,
                        ARTIFACT_THRESHOLD,
                    ) {
                        if let Some(obj) = result.metadata.as_object_mut() {
                            obj.insert("artifactId".to_string(), json!(artifact.id));
                            obj.insert("artifactKind".to_string(), json!("tool_output"));
                            obj.insert(
                                "artifact".to_string(),
                                json!({
                                    "id": artifact.id,
                                    "kind": "tool_output",
                                    "bytes": artifact_result.output.len()
                                }),
                            );
                        }
                        result
                            .output
                            .push_str(&format!("\n\n[Artifact: {}]", artifact.id));
                    }
                }
            }
        }

        Ok(result)
    }
}

pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show the current git diff for the workspace."
    }

    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let output = ctx
            .environment
            .exec(
                ExecRequest::new("git").args(["diff", "--"]),
                ctx.execution_context(Duration::from_secs(20)),
            )
            .await
            .context("git diff failed")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let text = if stdout.trim().is_empty() {
            format!(
                "[stdout]\n(no diff)\n\n[stderr]\n{}",
                truncate(&stderr, 8_000)
            )
        } else {
            truncate(&stdout, 32_000)
        };
        Ok(ToolResult {
            call_id: call.id,
            output: text,
            content: Vec::new(),
            metadata: json!({
                "exitCode": output.exit_code,
                "success": output.success,
                "sandbox": output.sandbox
            }),
        })
    }
}

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to files in the workspace using git apply."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string", "description": "Unified diff patch." }
            },
            "required": ["patch"]
        })
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let patch = call
            .input
            .get("patch")
            .and_then(Value::as_str)
            .context("apply_patch requires a patch")?;

        enforce_policy_decision(
            ctx.policy
                .inspect_command("git apply --whitespace=nowarn -"),
            ctx.approval_granted,
        )?;

        let result = ctx
            .environment
            .apply_patch(patch, ctx.execution_context(Duration::from_secs(30)))
            .await
            .context("git apply failed")?;
        let output = result.exec;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.success && looks_like_sandbox_denial(&stderr) {
            anyhow::bail!(
                "approval required: patch was blocked by the sandbox: {}",
                truncate(&stderr, 2_000)
            );
        }
        if !output.success {
            anyhow::bail!(
                "git apply failed ({:?})\n{}",
                output.exit_code,
                truncate(&stderr, 12_000)
            );
        }

        Ok(ToolResult {
            call_id: call.id,
            output: format!(
                "Patch applied.\n\n[stdout]\n{}\n\n[stderr]\n{}",
                truncate(&stdout, 8_000),
                truncate(&stderr, 8_000)
            ),
            content: Vec::new(),
            metadata: json!({
                "success": true,
                "bytes": result.bytes,
                "sandbox": output.sandbox
            }),
        })
    }
}

fn normalize_workspace_path(workspace_root: &Path, path: &str) -> anyhow::Result<PathBuf> {
    let candidate = PathBuf::from(path);
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!(
            "workspace path cannot contain '..': {}",
            candidate.display()
        );
    }
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Ok(workspace_root.join(candidate))
    }
}

fn enforce_read_policy(ctx: &ToolContext, path: &Path) -> anyhow::Result<()> {
    enforce_policy_decision(ctx.policy.inspect_read(path), ctx.approval_granted)
}

fn looks_like_sandbox_denial(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    [
        "access is denied",
        "access denied",
        "access to the path",
        "permissiondenied",
        "permission denied",
        "operation not permitted",
        "read-only file system",
        "unauthorized",
        "unauthorizedaccessexception",
        "network is unreachable",
        "network access is denied",
        "blocked by sandbox",
    ]
    .iter()
    .any(|pattern| stderr.contains(pattern))
}

struct SearchRun {
    engine: &'static str,
    output: String,
    matches: usize,
    returned_matches: usize,
    truncated: bool,
    original_bytes: usize,
    output_bytes: usize,
    fallback: Value,
}

struct FallbackCollector {
    lines: Vec<String>,
    matches: usize,
    original_bytes: usize,
    files_scanned: usize,
    files_skipped: usize,
    policy_skipped: usize,
    max_results: usize,
}

impl FallbackCollector {
    fn new(max_results: usize) -> Self {
        Self {
            lines: Vec::new(),
            matches: 0,
            original_bytes: 0,
            files_scanned: 0,
            files_skipped: 0,
            policy_skipped: 0,
            max_results,
        }
    }

    fn push_match(&mut self, line: String) {
        self.matches += 1;
        self.original_bytes += line.len() + 1;
        if self.lines.len() < self.max_results {
            self.lines.push(line);
        }
    }
}

async fn run_rg_search(
    environment: &dyn ExecutionEnvironment,
    search_path: &Path,
    query: &str,
    max_results: usize,
) -> anyhow::Result<Option<SearchRun>> {
    let output = match environment
        .exec(
            ExecRequest::new("rg").args([
                "--line-number".to_string(),
                "--column".to_string(),
                "--color".to_string(),
                "never".to_string(),
                "--no-heading".to_string(),
                "--no-messages".to_string(),
                "--max-count".to_string(),
                max_results.to_string(),
                "--".to_string(),
                query.to_string(),
                search_path.to_string_lossy().into_owned(),
            ]),
            ExecutionContext::with_timeout(Duration::from_secs(30)),
        )
        .await
    {
        Ok(output) => output,
        Err(err) if is_not_found_error(&err) => return Ok(None),
        Err(err) => return Err(err).context("failed to run rg search"),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.success && output.exit_code != Some(1) {
        anyhow::bail!(
            "rg search failed ({:?})\n{}",
            output.exit_code,
            truncate(&stderr, 12_000)
        );
    }

    Ok(Some(finalize_search_run(
        "rg",
        stdout.lines().map(str::to_string).collect(),
        stdout.lines().count(),
        stdout.len(),
        max_results,
        json!({ "used": false, "sandbox": output.sandbox }),
    )))
}

async fn run_fallback_search(
    workspace_root: PathBuf,
    search_path: PathBuf,
    policy: Arc<dyn PolicyEngine>,
    query: String,
    max_results: usize,
) -> anyhow::Result<SearchRun> {
    tokio::task::spawn_blocking(move || {
        let mut collector = FallbackCollector::new(max_results);
        collect_fallback_search(
            &workspace_root,
            &search_path,
            policy.as_ref(),
            &query,
            &mut collector,
        )?;
        let fallback = json!({
            "used": true,
            "mode": "substring",
            "maxFileBytes": FALLBACK_MAX_FILE_BYTES,
            "filesScanned": collector.files_scanned,
            "filesSkipped": collector.files_skipped,
            "policySkipped": collector.policy_skipped
        });
        Ok(finalize_search_run(
            "fallback-substring",
            collector.lines,
            collector.matches,
            collector.original_bytes,
            max_results,
            fallback,
        ))
    })
    .await
    .context("fallback search task failed")?
}

fn collect_fallback_search(
    workspace_root: &Path,
    path: &Path,
    policy: &dyn PolicyEngine,
    query: &str,
    collector: &mut FallbackCollector,
) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        collector.files_skipped += 1;
        return Ok(());
    }

    if metadata.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .with_context(|| format!("failed to list {}", path.display()))?
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            collect_fallback_search(workspace_root, &entry.path(), policy, query, collector)?;
        }
        return Ok(());
    }

    if !metadata.is_file() {
        collector.files_skipped += 1;
        return Ok(());
    }

    match policy.inspect_read(path) {
        PolicyDecision::Allow => {}
        PolicyDecision::Deny { .. } | PolicyDecision::Ask { .. } => {
            collector.policy_skipped += 1;
            return Ok(());
        }
    }

    if metadata.len() > FALLBACK_MAX_FILE_BYTES {
        collector.files_skipped += 1;
        return Ok(());
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(_) => {
            collector.files_skipped += 1;
            return Ok(());
        }
    };
    collector.files_scanned += 1;

    let display_path = display_workspace_path(workspace_root, path);
    for (line_index, line) in contents.lines().enumerate() {
        if let Some(byte_index) = line.find(query) {
            let column = line[..byte_index].chars().count() + 1;
            collector.push_match(format!(
                "{}:{}:{}:{}",
                display_path,
                line_index + 1,
                column,
                line
            ));
        }
    }

    Ok(())
}

fn finalize_search_run(
    engine: &'static str,
    lines: Vec<String>,
    matches: usize,
    original_bytes: usize,
    max_results: usize,
    fallback: Value,
) -> SearchRun {
    let returned_matches = lines.len().min(max_results);
    let text = if lines.is_empty() {
        "(no matches)".to_string()
    } else {
        lines
            .into_iter()
            .take(max_results)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let line_truncated = matches > max_results;
    let (output, byte_truncated) = truncate_bytes(&text, SEARCH_OUTPUT_MAX_BYTES);
    let output_bytes = output.len();
    SearchRun {
        engine,
        output,
        matches,
        returned_matches,
        truncated: line_truncated || byte_truncated,
        original_bytes,
        output_bytes,
        fallback,
    }
}

fn truncate_bytes(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = value[..end].to_string();
    truncated.push_str("\n\n[output truncated]");
    (truncated, true)
}

fn search_command_path(relative: &str, normalized: &Path) -> PathBuf {
    let candidate = PathBuf::from(relative);
    if candidate.is_absolute() {
        normalized.to_path_buf()
    } else {
        candidate
    }
}

fn display_workspace_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn list_dir_entries(path: &Path) -> anyhow::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in
        std::fs::read_dir(path).with_context(|| format!("failed to list {}", path.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let marker = if file_type.is_dir() { "/" } else { "" };
        entries.push(format!("{}{}", entry.file_name().to_string_lossy(), marker));
    }
    entries.sort();
    Ok(entries)
}

fn is_not_found_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|err| err.kind() == ErrorKind::NotFound)
    })
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("\n\n[output truncated]");
    truncated
}

pub struct McpToolWrapper {
    host: McpExtensionHost,
    descriptor: McpToolDescriptor,
}

impl McpToolWrapper {
    pub fn new(host: McpExtensionHost, descriptor: McpToolDescriptor) -> Self {
        Self { host, descriptor }
    }

    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.descriptor.public_name
    }

    fn description(&self) -> &str {
        self.descriptor.description.as_deref().unwrap_or_default()
    }

    fn schema(&self) -> Value {
        self.descriptor.input_schema.clone()
    }

    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult> {
        let permission = ToolPermissionDescriptor::from(&self.descriptor);
        enforce_policy_decision(
            ctx.policy.inspect_mcp_tool_call(&permission),
            ctx.approval_granted,
        )?;

        let result: McpCallResult = self
            .host
            .call_tool(&self.descriptor.public_name, call.input)
            .await?;
        let content = mcp_content_parts(&result.content, result.structured_content.as_ref());

        Ok(ToolResult {
            call_id: call.id,
            output: result.output,
            content,
            metadata: json!({
                "isError": result.is_error,
                "publicName": result.public_name,
                "toolName": result.tool_name,
                "serverId": result.server_id,
                "raw": result.raw,
            }),
        })
    }
}

fn mcp_content_parts(
    content: &[Value],
    structured_content: Option<&Value>,
) -> Vec<ModelContentPart> {
    let mut parts = Vec::new();
    for item in content {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(ModelContentPart::text(text));
                } else {
                    parts.push(ModelContentPart::json(item.clone()));
                }
            }
            Some("image") => {
                let content_type = item
                    .get("mimeType")
                    .or_else(|| item.get("mime_type"))
                    .and_then(Value::as_str);
                let data = item.get("data").and_then(Value::as_str);
                match (content_type, data.and_then(decode_mcp_base64)) {
                    (Some(content_type), Some(data)) => {
                        parts.push(ModelContentPart::image(content_type, data));
                    }
                    _ => parts.push(ModelContentPart::json(item.clone())),
                }
            }
            Some("resource") => {
                let resource = item.get("resource").unwrap_or(item);
                let uri = resource.get("uri").and_then(Value::as_str);
                if let Some(uri) = uri {
                    parts.push(ModelContentPart::resource(
                        uri,
                        resource
                            .get("mimeType")
                            .or_else(|| resource.get("mime_type"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        resource
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ));
                    if let Some(text) = resource.get("text").and_then(Value::as_str) {
                        parts.push(ModelContentPart::text(text));
                    }
                } else {
                    parts.push(ModelContentPart::json(item.clone()));
                }
            }
            _ => parts.push(ModelContentPart::json(item.clone())),
        }
    }
    if let Some(value) = structured_content {
        parts.push(ModelContentPart::json(value.clone()));
    }
    parts
}

fn decode_mcp_base64(value: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let first = sextet(chunk[0])?;
        let second = sextet(chunk[1])?;
        let third = if chunk[2] == b'=' {
            None
        } else {
            Some(sextet(chunk[2])?)
        };
        let fourth = if chunk[3] == b'=' {
            None
        } else {
            Some(sextet(chunk[3])?)
        };
        if third.is_none() && fourth.is_some() {
            return None;
        }
        decoded.push(first << 2 | second >> 4);
        if let Some(third) = third {
            decoded.push((second & 0b0000_1111) << 4 | third >> 2);
            if let Some(fourth) = fourth {
                decoded.push((third & 0b0000_0011) << 6 | fourth);
            }
        }
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Approval;
    use crate::policy::{BasicPolicyEngine, PermissionMode};
    use crate::store::SqliteSessionStore;
    use crate::subagents::{
        NoopSubagentObserver, SubagentExecutor, SubagentRun, SubagentSchedulerConfig,
    };
    use tokio::sync::mpsc;

    struct PendingExecutor;

    struct ImmediateExecutor;

    #[async_trait]
    impl SubagentExecutor for PendingExecutor {
        async fn execute(
            &self,
            _run: SubagentRun,
            _input: mpsc::UnboundedReceiver<String>,
            cancellation: CancellationToken,
        ) -> anyhow::Result<String> {
            cancellation.cancelled().await;
            anyhow::bail!("cancelled")
        }
    }

    #[async_trait]
    impl SubagentExecutor for ImmediateExecutor {
        async fn execute(
            &self,
            run: SubagentRun,
            _input: mpsc::UnboundedReceiver<String>,
            _cancellation: CancellationToken,
        ) -> anyhow::Result<String> {
            Ok(format!("completed {}", run.input))
        }
    }

    fn test_scheduler() -> SubagentScheduler {
        SubagentScheduler::new(
            SubagentSchedulerConfig {
                max_concurrency_per_parent: 1,
                max_depth: 2,
                timeout: Some(Duration::from_secs(10)),
            },
            Arc::new(PendingExecutor),
            Arc::new(NoopSubagentObserver),
        )
    }

    fn tool_context(
        scheduler: SubagentScheduler,
        thread_id: Uuid,
        parent_turn_id: Uuid,
    ) -> ToolContext {
        let workspace_root = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let mut context = ToolContext::local(workspace_root, policy);
        context.subagents = Some(scheduler);
        context.thread_id = Some(thread_id);
        context.parent_turn_id = Some(parent_turn_id);
        context
    }

    #[test]
    fn detects_common_cross_platform_sandbox_denials() {
        assert!(looks_like_sandbox_denial("Access is denied."));
        assert!(looks_like_sandbox_denial(
            "Access to the path 'C:\\\\outside.txt' is denied."
        ));
        assert!(looks_like_sandbox_denial("CategoryInfo: PermissionDenied"));
        assert!(looks_like_sandbox_denial("bash: Permission denied"));
        assert!(looks_like_sandbox_denial("Operation not permitted"));
        assert!(looks_like_sandbox_denial("Network is unreachable"));
        assert!(!looks_like_sandbox_denial("cargo test failed"));
    }

    #[tokio::test]
    async fn file_observation_tools_reject_parent_traversal_and_absolute_parent_paths() {
        let id = Uuid::new_v4();
        let workspace_root = std::env::temp_dir().join(format!("opentopia-tools-root-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-tools-outside-{id}"));
        fs::create_dir_all(&workspace_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), "outside marker").unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let mut context = ToolContext::local(workspace_root.clone(), policy);

        let traversal = ListFilesTool
            .execute(
                ToolCall::new("list_files", json!({ "path": "../.." })),
                context.clone(),
            )
            .await
            .unwrap_err();
        assert!(traversal.to_string().contains("cannot contain '..'"));

        let outside_path = outside.display().to_string();
        let approval_error = ReadFileTool
            .execute(
                ToolCall::new(
                    "read_file",
                    json!({ "path": outside.join("secret.txt").display().to_string() }),
                ),
                context.clone(),
            )
            .await
            .unwrap_err();
        assert!(approval_error
            .to_string()
            .contains("no readable root authorized"));

        context.approval_granted = true;
        let list_error = ListFilesTool
            .execute(
                ToolCall::new("list_files", json!({ "path": outside_path })),
                context.clone(),
            )
            .await
            .unwrap_err();
        assert!(list_error
            .to_string()
            .contains("no readable root authorized"));

        let read_error = ReadFileTool
            .execute(
                ToolCall::new(
                    "read_file",
                    json!({ "path": outside.join("secret.txt").display().to_string() }),
                ),
                context.clone(),
            )
            .await
            .unwrap_err();
        assert!(read_error
            .to_string()
            .contains("no readable root authorized"));

        let search_error = SearchTool
            .execute(
                ToolCall::new(
                    "search",
                    json!({ "query": "marker", "path": outside.display().to_string() }),
                ),
                context,
            )
            .await
            .unwrap_err();
        assert!(search_error
            .to_string()
            .contains("no readable root authorized"));

        fs::remove_dir_all(workspace_root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[tokio::test]
    async fn file_observation_tools_preserve_explicit_additional_readable_roots() {
        let id = Uuid::new_v4();
        let workspace_root = std::env::temp_dir().join(format!("opentopia-tools-root-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-tools-readable-{id}"));
        fs::create_dir_all(&workspace_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("allowed.txt"), "configured marker").unwrap();
        let mut config = LocalSandboxConfig::default();
        config.read_paths = vec![outside.clone()];
        let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
            workspace_root.clone(),
            PermissionMode::Auto,
            &config,
        ));
        let context =
            ToolContext::local_with_sandbox_config(workspace_root.clone(), policy, config);

        let listed = ListFilesTool
            .execute(
                ToolCall::new(
                    "list_files",
                    json!({ "path": outside.display().to_string() }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        assert!(listed.output.contains("allowed.txt"));

        let read = ReadFileTool
            .execute(
                ToolCall::new(
                    "read_file",
                    json!({ "path": outside.join("allowed.txt").display().to_string() }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        assert!(read.output.contains("configured marker"));

        let searched = SearchTool
            .execute(
                ToolCall::new(
                    "search",
                    json!({ "query": "configured marker", "path": outside.display().to_string() }),
                ),
                context,
            )
            .await
            .unwrap();
        assert!(searched.output.contains("configured marker"));

        fs::remove_dir_all(workspace_root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[tokio::test]
    async fn write_file_preserves_explicit_additional_writable_roots() {
        let id = Uuid::new_v4();
        let workspace_root = std::env::temp_dir().join(format!("opentopia-tools-root-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-tools-writable-{id}"));
        fs::create_dir_all(&workspace_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let mut config = LocalSandboxConfig::default();
        config.writable_roots = vec![outside.clone()];
        let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
            workspace_root.clone(),
            PermissionMode::Auto,
            &config,
        ));
        let context =
            ToolContext::local_with_sandbox_config(workspace_root.clone(), policy, config);
        let target = outside.join("dependency-cache.txt");

        WriteFileTool
            .execute(
                ToolCall::new(
                    "write_file",
                    json!({
                        "path": target.display().to_string(),
                        "content": "configured writable root"
                    }),
                ),
                context,
            )
            .await
            .expect("configured writable root should not require approval");
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "configured writable root"
        );

        fs::remove_dir_all(workspace_root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn browser_domain_grants_are_thread_scoped_and_normalized() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let first_thread = store
            .create_thread(Some("first".to_string()), PathBuf::from("."))
            .expect("create first thread");
        let second_thread = store
            .create_thread(Some("second".to_string()), PathBuf::from("."))
            .expect("create second thread");
        let host =
            browser_domain_from_url("https://Example.COM:8443/path").expect("parse browser URL");
        assert_eq!(host, "example.com");

        let approval = Approval::pending(
            Uuid::new_v4(),
            first_thread.id,
            browser_domain_approval_action(&host),
            "test domain approval",
        );
        let approval_id = approval.approval_id;
        store.insert_approval(approval).expect("persist approval");
        store
            .update_approval_status(approval_id, ApprovalStatus::Approved)
            .expect("approve domain");

        assert!(
            browser_domain_is_approved(&store, first_thread.id, "EXAMPLE.COM.")
                .expect("read grant")
        );
        assert!(!browser_domain_is_approved(&store, second_thread.id, &host)
            .expect("grants do not cross threads"));
    }

    #[test]
    fn preserves_typed_mcp_content_and_structured_content() {
        let parts = mcp_content_parts(
            &[
                json!({ "type": "text", "text": "observed" }),
                json!({
                    "type": "image",
                    "mimeType": "image/png",
                    "data": "iVBORw=="
                }),
                json!({
                    "type": "resource",
                    "resource": {
                        "uri": "file:///workspace/report.pdf",
                        "mimeType": "application/pdf",
                        "name": "report.pdf",
                        "text": "First page"
                    }
                }),
            ],
            Some(&json!({ "count": 1 })),
        );

        assert_eq!(parts[0], ModelContentPart::text("observed"));
        assert_eq!(
            parts[1],
            ModelContentPart::image("image/png", vec![0x89, b'P', b'N', b'G'])
        );
        assert_eq!(
            parts[2],
            ModelContentPart::resource(
                "file:///workspace/report.pdf",
                Some("application/pdf".to_string()),
                Some("report.pdf".to_string()),
            )
        );
        assert_eq!(parts[3], ModelContentPart::text("First page"));
        assert_eq!(parts[4], ModelContentPart::json(json!({ "count": 1 })));
    }

    #[test]
    fn rejects_invalid_mcp_base64_without_losing_the_original_json() {
        assert_eq!(decode_mcp_base64("not-base64"), None);
        let parts = mcp_content_parts(
            &[json!({ "type": "image", "mimeType": "image/png", "data": "bad" })],
            None,
        );
        assert_eq!(
            parts,
            vec![ModelContentPart::json(json!({
                "type": "image",
                "mimeType": "image/png",
                "data": "bad"
            }))]
        );
    }

    #[tokio::test]
    async fn model_subagent_tools_enforce_thread_and_parent_scope() {
        let scheduler = test_scheduler();
        let target_thread = Uuid::new_v4();
        let target_parent = Uuid::new_v4();
        let run = scheduler
            .spawn(SpawnSubagentRequest {
                parent_thread_id: target_thread,
                parent_turn_id: target_parent,
                name: "owned".to_string(),
                input: "work".to_string(),
                depth: 1,
            })
            .unwrap();

        let cross_thread = tool_context(scheduler.clone(), Uuid::new_v4(), target_parent);
        let error = SendAgentInputTool
            .execute(
                ToolCall::new("send_input", json!({ "runId": run.id, "input": "intrude" })),
                cross_thread,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("subagent run not found"));

        let wrong_parent = tool_context(scheduler.clone(), target_thread, Uuid::new_v4());
        let error = WaitAgentTool
            .execute(
                ToolCall::new("wait_agent", json!({ "runId": run.id, "timeoutMs": 5 })),
                wrong_parent,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("subagent run not found"));

        scheduler.cancel(run.id).unwrap();
    }

    #[tokio::test]
    async fn complete_task_returns_a_structured_terminal_signal() {
        let workspace_root = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let result = CompleteTaskTool
            .execute(
                ToolCall::new(
                    "complete_task",
                    json!({
                        "summary": "Requested scope is complete.",
                        "verification": ["Focused tests passed"],
                        "remaining_work": ["A later phase remains pending"]
                    }),
                ),
                ToolContext::local(workspace_root.clone(), policy.clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.metadata["success"], true);
        assert_eq!(
            result.metadata["taskCompletion"]["summary"],
            "Requested scope is complete."
        );
        assert!(result.output.contains("Focused tests passed"));
        assert!(result.output.contains("A later phase remains pending"));

        let invalid = CompleteTaskTool
            .execute(
                ToolCall::new(
                    "complete_task",
                    json!({
                        "summary": "   ",
                        "verification": [],
                        "remaining_work": []
                    }),
                ),
                ToolContext::local(workspace_root, policy),
            )
            .await
            .unwrap_err();
        assert!(invalid.to_string().contains("summary cannot be empty"));
    }

    #[tokio::test]
    async fn update_plan_validates_progress_and_parent_ownership() {
        let workspace_root = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let context = ToolContext::local(workspace_root.clone(), policy.clone());
        let result = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "explanation": "Track the work",
                        "plan": [
                            { "step": "Inspect inputs", "status": "completed" },
                            { "step": "Produce output", "status": "in_progress" }
                        ]
                    }),
                ),
                context,
            )
            .await
            .unwrap();
        let plan: TaskPlan = serde_json::from_value(result.metadata["taskPlan"].clone()).unwrap();
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.is_active());

        let completed_scope = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "current_scope_complete": true,
                        "verification": ["npm test passed"],
                        "plan": [
                            { "step": "Current phase", "status": "completed" },
                            { "step": "Later phase", "status": "pending" }
                        ]
                    }),
                ),
                ToolContext::local(workspace_root.clone(), policy.clone()),
            )
            .await
            .unwrap();
        assert_eq!(completed_scope.metadata["currentScopeComplete"], true);
        assert_eq!(completed_scope.metadata["allStepsComplete"], false);
        assert_eq!(
            completed_scope.metadata["verification"][0],
            "npm test passed"
        );

        let invalid = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "plan": [
                            { "step": "First", "status": "in_progress" },
                            { "step": "Second", "status": "in_progress" }
                        ]
                    }),
                ),
                ToolContext::local(workspace_root.clone(), policy.clone()),
            )
            .await
            .unwrap_err();
        assert!(invalid.to_string().contains("at most one in_progress"));

        let mut child_context = ToolContext::local(workspace_root, policy);
        child_context.subagent_depth = 1;
        let denied = UpdatePlanTool
            .execute(
                ToolCall::new("update_plan", json!({ "plan": [] })),
                child_context,
            )
            .await
            .unwrap_err();
        assert!(denied.to_string().contains("only the parent agent"));
    }

    #[tokio::test]
    async fn wait_agents_collects_parallel_child_results() {
        let scheduler = SubagentScheduler::new(
            SubagentSchedulerConfig {
                max_concurrency_per_parent: 2,
                max_depth: 2,
                timeout: Some(Duration::from_secs(10)),
            },
            Arc::new(ImmediateExecutor),
            Arc::new(NoopSubagentObserver),
        );
        let thread_id = Uuid::new_v4();
        let parent_turn_id = Uuid::new_v4();
        let first = scheduler
            .spawn(SpawnSubagentRequest {
                parent_thread_id: thread_id,
                parent_turn_id,
                name: "first".to_string(),
                input: "alpha".to_string(),
                depth: 1,
            })
            .unwrap();
        let second = scheduler
            .spawn(SpawnSubagentRequest {
                parent_thread_id: thread_id,
                parent_turn_id,
                name: "second".to_string(),
                input: "beta".to_string(),
                depth: 1,
            })
            .unwrap();

        let result = WaitAgentsTool
            .execute(
                ToolCall::new(
                    "wait_agents",
                    json!({
                        "runIds": [first.id, second.id],
                        "timeoutMs": 1_000
                    }),
                ),
                tool_context(scheduler, thread_id, parent_turn_id),
            )
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(value["allTerminal"], true);
        assert_eq!(value["allSucceeded"], true);
        assert_eq!(value["runs"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn spreadsheet_tool_round_trips_through_execution_environment() {
        let workspace_root =
            std::env::temp_dir().join(format!("opentopia-sheet-{}", Uuid::new_v4()));
        fs::create_dir_all(&workspace_root).unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let context = ToolContext::local(workspace_root.clone(), policy.clone());
        let written = SpreadsheetTool
            .execute(
                ToolCall::new(
                    "spreadsheet",
                    json!({
                        "action": "write",
                        "outputPath": "report.xlsx",
                        "sheets": [{
                            "name": "Summary",
                            "cells": [{
                                "address": { "row": 0, "column": 0 },
                                "value": { "type": "string", "value": "ready" }
                            }]
                        }]
                    }),
                ),
                context,
            )
            .await
            .unwrap();
        assert_eq!(written.metadata["success"], true);
        assert!(workspace_root.join("report.xlsx").is_file());

        let read = SpreadsheetTool
            .execute(
                ToolCall::new(
                    "spreadsheet",
                    json!({
                        "action": "read_range",
                        "path": "report.xlsx",
                        "sheet": "Summary",
                        "range": {
                            "start": { "row": 0, "column": 0 },
                            "end": { "row": 0, "column": 0 }
                        }
                    }),
                ),
                ToolContext::local(workspace_root.clone(), policy),
            )
            .await
            .unwrap();
        assert!(read.output.contains("ready"));
        fs::remove_dir_all(workspace_root).unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn write_file_allows_verbatim_workspace_target_in_approve_mode() {
        let workspace_root = std::env::temp_dir().join(format!(
            "opentopia-write-verbatim-workspace-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(workspace_root.join("design")).expect("create workspace fixture");
        let verbatim_root = workspace_root.canonicalize().expect("canonical workspace");
        assert!(verbatim_root.to_string_lossy().starts_with(r"\\?\"));
        let target = verbatim_root.join("design/requirements.md");
        let policy = Arc::new(BasicPolicyEngine::new(
            verbatim_root.clone(),
            PermissionMode::Approve,
        ));
        let context = ToolContext::local_with_sandbox_config(
            verbatim_root,
            policy,
            LocalSandboxConfig::default(),
        );

        let result = WriteFileTool
            .execute(
                ToolCall::new(
                    "write_file",
                    json!({
                        "path": target.display().to_string(),
                        "content": "workspace write is authorized"
                    }),
                ),
                context,
            )
            .await
            .expect("workspace write must not require approval");

        assert_eq!(result.metadata["changedPath"], target.display().to_string());
        assert_eq!(
            fs::read_to_string(&target).expect("read written fixture"),
            "workspace write is authorized"
        );
        fs::remove_dir_all(workspace_root).expect("remove workspace fixture");
    }
}
