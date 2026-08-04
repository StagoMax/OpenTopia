use crate::agent_profiles::AgentProfileRegistry;
use crate::background::{
    BackgroundProcessRegistry, BackgroundScope, BackgroundSessionSpawnRequest,
    BackgroundSpawnRequest,
};
use crate::browser::{
    BrowserAction, BrowserActionReceipt, BrowserContent, BrowserDownloadRequest,
    BrowserNavigateRequest, BrowserNodeRef, BrowserObservation, BrowserObservationId,
    BrowserObserveOptions, BrowserRuntime, BrowserSelector, BrowserSessionId, BrowserWaitCondition,
    BrowserWaitRequest,
};
use crate::bundled_plugins::bundled_plugin_catalog;
use crate::computer::{
    ComputerAction, ComputerMouseButton, ComputerPolicyContext, ComputerRuntime, ComputerSessionId,
    ObserveOptions,
};
use crate::enterprise::{CapabilityProjection, DataClassification};
use crate::execution::{
    ExecRequest, ExecutionContext, ExecutionEnvironment, FileDeleteRequest, FileReadRequest,
    FileWriteRequest, LocalExecutionEnvironment,
};
use crate::git_workflow::isolated_subagent_worktree_request;
use crate::mcp::{McpCallResult, McpToolDescriptor};
use crate::mcp_host::McpExtensionHost;
use crate::model::{
    CollaborationMode, ModelContentPart, TaskPlan, TaskPlanStep, TaskPlanStepStatus, ToolCall,
    ToolResult, UserInputOption, UserInputQuestion, UserInputRequest,
};
use crate::model_context::CompiledModelContext;
use crate::policy::{ApprovalRequired, PolicyDecision, PolicyEngine, ToolPermissionDescriptor};
use crate::provider::{ModelConversationMessage, ModelConversationRole};
use crate::sandbox::{LocalSandboxConfig, SandboxMode};
use crate::skill_authoring::{
    create_skill_from_draft, preview_skill_draft, SkillDraft, SkillResourceDraft,
};
use crate::skills::{discover_skills, load_skill_slice, SkillScope, MAX_SKILL_BYTES};
use crate::spreadsheet::{
    execute_spreadsheet, CellRange, InspectWorkbookRequest, ListSheetsRequest, ReadRangeRequest,
    SheetWriteRequest, SpreadsheetAction, SpreadsheetRequest, SpreadsheetResult,
    WriteWorkbookRequest, MAX_INPUT_FILE_BYTES as MAX_SPREADSHEET_INPUT_BYTES,
};
use crate::store::SessionStore;
use crate::subagents::{
    SpawnSubagentRequest, SubagentExecutionContract, SubagentRun, SubagentRunStatus,
    SubagentScheduler, SubagentScope, SubagentWorkspaceAssignment, SubagentWorkspaceMode,
};
use anyhow::Context;
use async_trait::async_trait;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use schemars::{gen::SchemaSettings, JsonSchema};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
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
    /// Commands that outlive the tool call that started them.
    pub background: Option<BackgroundProcessRegistry>,
    pub parent_turn_id: Option<Uuid>,
    pub subagent_depth: u8,
    pub agent_path: String,
    pub browser: Option<Arc<dyn BrowserRuntime>>,
    pub computer: Option<Arc<dyn ComputerRuntime>>,
    pub fork_conversation: Vec<ModelConversationMessage>,
    pub fork_model_context: Option<CompiledModelContext>,
    pub current_task_plan: Option<TaskPlan>,
    pub collaboration_mode: CollaborationMode,
    pub goal_id: Option<Uuid>,
    /// Set only while replaying a tool call that the user explicitly approved.
    /// Browser navigation uses this as a one-time fallback when a caller does not have a
    /// persistent session store from which it can read the approved domain.
    pub approval_granted: bool,
    /// The same fail-closed projection used to build the provider catalog.
    /// Discovery tools must apply it to their result contents as well.
    pub capability_projection: CapabilityProjection,
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
            background: None,
            parent_turn_id: None,
            subagent_depth: 0,
            agent_path: "/root".to_string(),
            browser: None,
            computer: None,
            fork_conversation: Vec::new(),
            fork_model_context: None,
            current_task_plan: None,
            collaboration_mode: CollaborationMode::Default,
            goal_id: None,
            approval_granted: false,
            capability_projection: CapabilityProjection::unrestricted(),
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
            background: None,
            parent_turn_id: None,
            subagent_depth: 0,
            agent_path: "/root".to_string(),
            browser: None,
            computer: None,
            fork_conversation: Vec::new(),
            fork_model_context: None,
            current_task_plan: None,
            collaboration_mode: CollaborationMode::Default,
            goal_id: None,
            approval_granted: false,
            capability_projection: CapabilityProjection::unrestricted(),
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
        PolicyDecision::Ask { reason } => Err(ApprovalRequired::new(reason).into()),
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;
    fn input_error(&self, input: &Value) -> Option<String> {
        crate::provider::tool_input_schema_error(&self.schema(), input, "arguments")
    }
    fn has_derived_input_schema(&self) -> bool {
        false
    }
    /// Scheduling facts, separate from the model-facing schema. AgentCore may use
    /// these to run independent observations concurrently without guessing from
    /// tool names. The default is intentionally conservative for plugins/MCP.
    fn execution_policy(&self, _call: &ToolCall) -> ToolExecutionPolicy {
        ToolExecutionPolicy::conservative()
    }
    async fn execute(&self, call: ToolCall, ctx: ToolContext) -> anyhow::Result<ToolResult>;
}

/// Static tools declare their input exactly once. The associated Rust type is
/// used both to derive the model-facing JSON Schema and to deserialize the
/// value that reaches policy evaluation and execution. Tools whose contracts
/// are discovered dynamically (currently MCP) continue to implement `Tool`
/// directly.
#[async_trait]
trait TypedTool: Send + Sync {
    type Input: DeserializeOwned + JsonSchema + Send + 'static;

    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn validate_context(&self, _ctx: &ToolContext) -> anyhow::Result<()> {
        Ok(())
    }
    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::conservative()
    }
    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        _ctx: ToolContext,
    ) -> anyhow::Result<ToolResult>;
}

fn derived_tool_schema<T: JsonSchema>() -> Value {
    let mut settings = SchemaSettings::draft07();
    settings.inline_subschemas = true;
    settings.meta_schema = None;
    let schema = settings.into_generator().into_root_schema_for::<T>();
    let mut value = serde_json::to_value(schema).expect("derived tool schema must serialize");
    if let Some(object) = value.as_object_mut() {
        object.remove("title");
        object.remove("definitions");
        let object_union = ["anyOf", "oneOf"].iter().any(|keyword| {
            object
                .get(*keyword)
                .and_then(Value::as_array)
                .is_some_and(|branches| {
                    !branches.is_empty()
                        && branches.iter().all(|branch| {
                            branch.get("type").and_then(Value::as_str) == Some("object")
                        })
                })
        });
        if object.get("type").is_none() && object_union {
            // Function-tool transports require an object at the root even when
            // Draft 7 can infer it from an anyOf/oneOf of object variants.
            object.insert("type".to_string(), Value::String("object".to_string()));
        }
    }
    value
}

fn decode_typed_tool_input<T: DeserializeOwned>(
    tool_name: &str,
    input: Value,
) -> anyhow::Result<T> {
    serde_json::from_value(input)
        .with_context(|| format!("invalid arguments for tool `{tool_name}`"))
}

macro_rules! impl_typed_tool {
    ($tool:ty) => {
        #[async_trait]
        impl Tool for $tool {
            fn name(&self) -> &str {
                <Self as TypedTool>::name(self)
            }

            fn description(&self) -> &str {
                <Self as TypedTool>::description(self)
            }

            fn schema(&self) -> Value {
                derived_tool_schema::<<Self as TypedTool>::Input>()
            }

            fn input_error(&self, input: &Value) -> Option<String> {
                crate::provider::tool_input_schema_error(&self.schema(), input, "arguments")
                    .or_else(|| {
                        serde_json::from_value::<<Self as TypedTool>::Input>(input.clone())
                            .err()
                            .map(|error| {
                                format!("arguments do not match the derived input type: {error}")
                            })
                    })
            }

            fn has_derived_input_schema(&self) -> bool {
                true
            }

            fn execution_policy(&self, call: &ToolCall) -> ToolExecutionPolicy {
                decode_typed_tool_input::<<Self as TypedTool>::Input>(
                    <Self as TypedTool>::name(self),
                    call.input.clone(),
                )
                .map(|input| <Self as TypedTool>::execution_policy(self, &input))
                .unwrap_or_else(|_| ToolExecutionPolicy::conservative())
            }

            async fn execute(
                &self,
                call: ToolCall,
                ctx: ToolContext,
            ) -> anyhow::Result<ToolResult> {
                <Self as TypedTool>::validate_context(self, &ctx)?;
                let input = decode_typed_tool_input::<<Self as TypedTool>::Input>(
                    <Self as TypedTool>::name(self),
                    call.input,
                )?;
                <Self as TypedTool>::execute_typed(self, call.id, input, ctx).await
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffect {
    None,
    WorkspaceWrite,
    Process,
    SessionMutation,
    ControlPlane,
    External,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionPolicy {
    pub read_only: bool,
    pub idempotent: bool,
    pub parallel_safe: bool,
    pub side_effect: ToolSideEffect,
    /// Logical resources touched by this call. Equal keys conflict; `*` is a
    /// global barrier. Paths remain logical and are not trusted for authorization.
    pub resource_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRiskLevel {
    Low,
    Medium,
    High,
    Critical,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalMode {
    Never,
    PolicyControlled,
    Always,
}

/// Registry metadata is control-plane data. It is intentionally kept out of
/// the provider function schema so governance does not consume model tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCapabilityDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub source: String,
    pub risk: ToolRiskLevel,
    pub potential_side_effects: Vec<ToolSideEffect>,
    pub approval: ToolApprovalMode,
    pub max_data_classification: DataClassification,
}

impl ToolExecutionPolicy {
    pub fn conservative() -> Self {
        Self {
            read_only: false,
            idempotent: false,
            parallel_safe: false,
            side_effect: ToolSideEffect::Unknown,
            resource_keys: vec!["*".to_string()],
        }
    }

    pub fn read_only(resource_keys: Vec<String>) -> Self {
        Self {
            read_only: true,
            idempotent: true,
            parallel_safe: true,
            side_effect: ToolSideEffect::None,
            resource_keys,
        }
    }
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Arc<BTreeMap<String, Arc<dyn Tool>>>,
    sources: Arc<BTreeMap<String, ToolSource>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    Core,
    BundledPlugin { plugin_name: String },
    Mcp,
}

impl ToolRegistry {
    pub fn with_builtins() -> Self {
        let mut registry = Self::with_core_tools();
        registry.register_bundled_plugins();
        registry
    }

    pub fn with_core_tools() -> Self {
        let mut tools: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
        tools.insert("list_files".to_string(), Arc::new(ListFilesTool));
        tools.insert("read_file".to_string(), Arc::new(ReadFileTool));
        tools.insert("read_files".to_string(), Arc::new(ReadFilesTool));
        tools.insert("write_file".to_string(), Arc::new(WriteFileTool));
        tools.insert("search".to_string(), Arc::new(SearchTool));
        tools.insert("shell".to_string(), Arc::new(ShellTool));
        tools.insert(
            "background_output".to_string(),
            Arc::new(BackgroundOutputTool),
        );
        tools.insert("git_diff".to_string(), Arc::new(GitDiffTool));
        tools.insert("apply_patch".to_string(), Arc::new(ApplyPatchTool));
        tools.insert("spawn_agent".to_string(), Arc::new(SpawnAgentTool));
        tools.insert("send_message".to_string(), Arc::new(SendAgentMessageTool));
        tools.insert("followup_task".to_string(), Arc::new(FollowupAgentTaskTool));
        tools.insert("interrupt_agent".to_string(), Arc::new(InterruptAgentTool));
        tools.insert("list_agents".to_string(), Arc::new(ListAgentsTool));
        tools.insert("send_input".to_string(), Arc::new(SendAgentInputTool));
        tools.insert("cancel_agent".to_string(), Arc::new(CancelAgentTool));
        tools.insert("wait_agent".to_string(), Arc::new(WaitAgentTool));
        tools.insert("wait_agents".to_string(), Arc::new(WaitAgentsTool));
        tools.insert(
            "request_user_input".to_string(),
            Arc::new(RequestUserInputTool),
        );
        tools.insert("set_plan".to_string(), Arc::new(SetPlanTool));
        tools.insert("update_plan".to_string(), Arc::new(UpdatePlanTool));
        tools.insert("complete_task".to_string(), Arc::new(CompleteTaskTool));
        tools.insert("list_skills".to_string(), Arc::new(ListSkillsTool));
        tools.insert("read_skill".to_string(), Arc::new(ReadSkillTool));
        tools.insert("create_skill".to_string(), Arc::new(CreateSkillTool));
        let sources = tools
            .keys()
            .cloned()
            .map(|name| (name, ToolSource::Core))
            .collect();
        Self {
            tools: Arc::new(tools),
            sources: Arc::new(sources),
        }
    }

    fn register_bundled_plugins(&mut self) {
        for plugin in bundled_plugin_catalog() {
            for capability in plugin.native_capabilities {
                let tool: Arc<dyn Tool> = match *capability {
                    "browser" => Arc::new(BrowserTool),
                    "computer" => Arc::new(ComputerTool),
                    "spreadsheet" => Arc::new(SpreadsheetTool),
                    _ => continue,
                };
                self.insert_with_source(
                    (*capability).to_string(),
                    tool,
                    ToolSource::BundledPlugin {
                        plugin_name: plugin.name.to_string(),
                    },
                );
            }
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn insert(&mut self, name: String, tool: Arc<dyn Tool>) {
        self.insert_with_source(name, tool, ToolSource::Core);
    }

    pub fn insert_mcp(&mut self, name: String, tool: Arc<dyn Tool>) {
        self.insert_with_source(name, tool, ToolSource::Mcp);
    }

    fn insert_with_source(&mut self, name: String, tool: Arc<dyn Tool>, source: ToolSource) {
        let tools = Arc::make_mut(&mut self.tools);
        tools.insert(name.clone(), tool);
        Arc::make_mut(&mut self.sources).insert(name, source);
    }

    pub fn list(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    pub fn source(&self, name: &str) -> Option<ToolSource> {
        self.sources.get(name).cloned()
    }

    pub fn execution_policy(&self, name: &str, call: &ToolCall) -> Option<ToolExecutionPolicy> {
        self.tools.get(name).map(|tool| tool.execution_policy(call))
    }

    pub fn capability_catalog(&self) -> Vec<ToolCapabilityDescriptor> {
        self.tools
            .iter()
            .map(|(name, tool)| {
                let source = self.sources.get(name).cloned().unwrap_or(ToolSource::Core);
                let (risk, potential_side_effects, approval, max_data_classification) =
                    tool_governance_metadata(name, &source);
                ToolCapabilityDescriptor {
                    name: name.clone(),
                    description: tool.description().to_string(),
                    input_schema: tool.schema(),
                    source: match &source {
                        ToolSource::Core => "core".to_string(),
                        ToolSource::BundledPlugin { plugin_name } => {
                            format!("bundled_plugin:{plugin_name}")
                        }
                        ToolSource::Mcp => "mcp".to_string(),
                    },
                    risk,
                    potential_side_effects,
                    approval,
                    max_data_classification,
                }
            })
            .collect()
    }
}

fn tool_governance_metadata(
    name: &str,
    source: &ToolSource,
) -> (
    ToolRiskLevel,
    Vec<ToolSideEffect>,
    ToolApprovalMode,
    DataClassification,
) {
    if matches!(source, ToolSource::Mcp) {
        return (
            ToolRiskLevel::High,
            vec![ToolSideEffect::External],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        );
    }
    match name {
        "list_files" | "read_file" | "read_files" | "search" | "git_diff" | "background_output"
        | "list_agents" | "wait_agent" | "wait_agents" | "list_skills" | "read_skill" => (
            ToolRiskLevel::Low,
            vec![ToolSideEffect::None],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "write_file" | "apply_patch" | "create_skill" | "spreadsheet" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::WorkspaceWrite],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "shell" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::Process],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Restricted,
        ),
        "browser" | "computer" => (
            ToolRiskLevel::High,
            vec![ToolSideEffect::External],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        ),
        "spawn_agent" | "send_message" | "followup_task" | "interrupt_agent" | "send_input"
        | "cancel_agent" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::ControlPlane],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Confidential,
        ),
        "request_user_input" | "set_plan" | "update_plan" | "complete_task" => (
            ToolRiskLevel::Medium,
            vec![ToolSideEffect::SessionMutation],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Confidential,
        ),
        _ => (
            ToolRiskLevel::Unknown,
            vec![ToolSideEffect::Unknown],
            ToolApprovalMode::PolicyControlled,
            DataClassification::Public,
        ),
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SpreadsheetToolAction {
    Inspect,
    ListSheets,
    ReadRange,
    Write,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpreadsheetToolInput {
    action: SpreadsheetToolAction,
    /// Workspace-relative XLSX path for inspect/list/read.
    #[serde(default)]
    path: Option<String>,
    /// Worksheet name for read_range.
    #[serde(default)]
    sheet: Option<String>,
    /// Inclusive zero-based range for read_range.
    #[serde(default)]
    range: Option<CellRange>,
    /// Optional existing XLSX to rebuild before applying writes.
    #[serde(default)]
    source_path: Option<String>,
    /// Workspace-relative XLSX output path for write.
    #[serde(default)]
    output_path: Option<String>,
    #[serde(default)]
    #[schemars(length(max = 256))]
    sheets: Vec<SheetWriteRequest>,
}

pub struct SpreadsheetTool;

#[async_trait]
impl TypedTool for SpreadsheetTool {
    type Input = SpreadsheetToolInput;

    fn name(&self) -> &str {
        "spreadsheet"
    }

    fn description(&self) -> &str {
        "Inspect, list, read, create, or update bounded XLSX workbooks. Uses zero-based row and column coordinates; writes preserve values, formulas, sheet order, and visibility but not formatting or embedded workbook objects."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        match input.action {
            SpreadsheetToolAction::Inspect
            | SpreadsheetToolAction::ListSheets
            | SpreadsheetToolAction::ReadRange => {
                execute_spreadsheet_read(call_id, input, ctx).await
            }
            SpreadsheetToolAction::Write => execute_spreadsheet_write(call_id, input, ctx).await,
        }
    }
}

impl_typed_tool!(SpreadsheetTool);

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

const MAX_USER_INPUT_QUESTIONS: usize = 3;
const MAX_USER_INPUT_OPTIONS: usize = 4;
const MAX_USER_INPUT_ID_CHARS: usize = 64;
const MAX_USER_INPUT_HEADER_CHARS: usize = 24;
const MAX_USER_INPUT_QUESTION_CHARS: usize = 500;
const MAX_USER_INPUT_LABEL_CHARS: usize = 100;
const MAX_USER_INPUT_DESCRIPTION_CHARS: usize = 500;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RequestUserInputInput {
    /// One to three concise planning decisions.
    #[schemars(length(min = 1, max = 3))]
    questions: Vec<RequestUserInputQuestionInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RequestUserInputQuestionInput {
    /// Stable snake_case identifier.
    id: String,
    /// Short card heading.
    header: String,
    question: String,
    #[schemars(length(min = 2, max = 4))]
    options: Vec<RequestUserInputOptionInput>,
    /// Allow the user to enter a different answer. Defaults to true.
    #[serde(default = "default_allow_custom")]
    allow_custom: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RequestUserInputOptionInput {
    /// Stable snake_case identifier.
    id: String,
    label: String,
    description: String,
    #[serde(default)]
    recommended: bool,
}

fn default_allow_custom() -> bool {
    true
}

pub struct RequestUserInputTool;

#[async_trait]
impl TypedTool for RequestUserInputTool {
    type Input = RequestUserInputInput;

    fn name(&self) -> &str {
        "request_user_input"
    }

    fn description(&self) -> &str {
        "Pause plan generation and ask the user to choose between materially different approaches. Use one to three concise questions with two to four concrete options each. Mark at most one option per question as recommended."
    }

    fn validate_context(&self, ctx: &ToolContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.collaboration_mode == CollaborationMode::Plan,
            "request_user_input is only available in plan mode"
        );
        Ok(())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        _ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        anyhow::ensure!(
            !input.questions.is_empty() && input.questions.len() <= MAX_USER_INPUT_QUESTIONS,
            "request_user_input requires one to {MAX_USER_INPUT_QUESTIONS} questions"
        );

        let mut question_ids = HashSet::new();
        let mut questions = Vec::with_capacity(input.questions.len());
        for question in input.questions {
            let id = validate_user_input_id("question id", question.id)?;
            anyhow::ensure!(
                question_ids.insert(id.clone()),
                "duplicate question id: {id}"
            );
            let header =
                validate_user_input_text("header", question.header, MAX_USER_INPUT_HEADER_CHARS)?;
            let prompt = validate_user_input_text(
                "question",
                question.question,
                MAX_USER_INPUT_QUESTION_CHARS,
            )?;
            anyhow::ensure!(
                (2..=MAX_USER_INPUT_OPTIONS).contains(&question.options.len()),
                "question {id} requires two to {MAX_USER_INPUT_OPTIONS} options"
            );

            let mut option_ids = HashSet::new();
            let mut option_labels = HashSet::new();
            let mut recommended_count = 0usize;
            let mut options = Vec::with_capacity(question.options.len());
            for option in question.options {
                let option_id = validate_user_input_id("option id", option.id)?;
                anyhow::ensure!(
                    option_ids.insert(option_id.clone()),
                    "question {id} contains duplicate option id: {option_id}"
                );
                let label = validate_user_input_text(
                    "option label",
                    option.label,
                    MAX_USER_INPUT_LABEL_CHARS,
                )?;
                anyhow::ensure!(
                    option_labels.insert(label.to_lowercase()),
                    "question {id} contains duplicate option label: {label}"
                );
                let description = validate_user_input_text(
                    "option description",
                    option.description,
                    MAX_USER_INPUT_DESCRIPTION_CHARS,
                )?;
                recommended_count += usize::from(option.recommended);
                options.push(UserInputOption {
                    id: option_id,
                    label,
                    description,
                    recommended: option.recommended,
                });
            }
            anyhow::ensure!(
                recommended_count <= 1,
                "question {id} may have at most one recommended option"
            );
            questions.push(UserInputQuestion {
                id,
                header,
                question: prompt,
                options,
                allow_custom: question.allow_custom,
            });
        }

        let request = UserInputRequest {
            request_id: Uuid::new_v4(),
            questions,
        };
        Ok(ToolResult {
            call_id,
            output: format!(
                "Waiting for the user to answer {} planning decision(s).",
                request.questions.len()
            ),
            content: vec![ModelContentPart::json(json!({
                "status": "waiting_for_user_input",
                "requestId": request.request_id,
            }))],
            metadata: json!({
                "toolName": "request_user_input",
                "userInputRequest": request,
                "success": true,
            }),
        })
    }
}

impl_typed_tool!(RequestUserInputTool);

fn validate_user_input_id(field: &str, value: String) -> anyhow::Result<String> {
    let value = validate_user_input_text(field, value, MAX_USER_INPUT_ID_CHARS)?;
    anyhow::ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-')),
        "request_user_input {field} must contain only letters, numbers, underscores, or hyphens"
    );
    Ok(value)
}

fn validate_user_input_text(
    field: &str,
    value: String,
    max_chars: usize,
) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    anyhow::ensure!(
        !value.is_empty(),
        "request_user_input {field} cannot be empty"
    );
    anyhow::ensure!(
        value.chars().count() <= max_chars,
        "request_user_input {field} exceeds the {max_chars} character limit"
    );
    Ok(value)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompleteTaskInput {
    /// Concise description of the completed result.
    summary: String,
    /// Commands, checks, or observed results that verify the completed scope.
    #[schemars(length(max = 20))]
    verification: Vec<String>,
    /// Work intentionally left for a later phase. Empty means no known remaining work.
    #[schemars(length(max = 20))]
    remaining_work: Vec<String>,
}

pub struct CompleteTaskTool;

#[async_trait]
impl TypedTool for CompleteTaskTool {
    type Input = CompleteTaskInput;

    fn name(&self) -> &str {
        "complete_task"
    }

    fn description(&self) -> &str {
        "Finish the current user task after its requested scope has been verified. The plan records commitments and evidence, not a mandatory reasoning path. Provide a concise summary, concrete verification evidence, and any deliberately deferred work. This is the final tool call for the turn."
    }

    fn validate_context(&self, ctx: &ToolContext) -> anyhow::Result<()> {
        if ctx.collaboration_mode == CollaborationMode::Plan {
            anyhow::bail!("complete_task is unavailable in plan mode");
        }
        if ctx.collaboration_mode == CollaborationMode::Goal {
            let goal_id = ctx
                .goal_id
                .context("goal mode is missing a server-assigned goal id")?;
            let plan = ctx
                .current_task_plan
                .as_ref()
                .context("goal mode cannot complete before a plan exists")?;
            anyhow::ensure!(
                plan.goal_id == goal_id.to_string(),
                "current plan belongs to a different goal"
            );
            anyhow::ensure!(!plan.steps.is_empty(), "goal plan cannot be empty");
            anyhow::ensure!(
                !plan.has_actionable_steps(),
                "goal still contains pending or in_progress steps"
            );
            anyhow::ensure!(
                plan.steps.iter().all(|step| {
                    step.status != TaskPlanStepStatus::Completed || !step.evidence.is_empty()
                }),
                "every completed goal step must include verification evidence"
            );
        }
        Ok(())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        _ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
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
            call_id,
            output,
            content: vec![ModelContentPart::json(completion.clone())],
            metadata: json!({
                "toolName": "complete_task",
                "taskCompletion": completion,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(CompleteTaskTool);

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
const MAX_TASK_PLAN_ID_CHARS: usize = 100;
const MAX_TASK_PLAN_CHANGE_REASON_CHARS: usize = 2_000;
const MAX_TASK_PLAN_STATUS_REASON_CHARS: usize = 1_000;
const MAX_TASK_PLAN_STEP_ITEMS: usize = 20;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetPlanInput {
    /// Exact goal UUID assigned by the server.
    goal_id: String,
    #[schemars(range(min = 0))]
    expected_revision: u64,
    change_reason: String,
    #[schemars(length(min = 1, max = 20))]
    steps: Vec<SetPlanStepInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetPlanStepInput {
    id: String,
    title: String,
    #[schemars(length(max = 20))]
    dependencies: Vec<String>,
    #[schemars(length(min = 1, max = 20))]
    acceptance_criteria: Vec<String>,
}

pub struct SetPlanTool;

#[async_trait]
impl TypedTool for SetPlanTool {
    type Input = SetPlanInput;

    fn name(&self) -> &str {
        "set_plan"
    }

    fn description(&self) -> &str {
        "Atomically create or replace the dependency-aware external memory for the server-assigned goal. The plan records commitments, progress, and completion evidence; it does not prescribe a fixed execution schedule beyond explicit dependencies. Every step starts pending and may be revised as evidence changes."
    }

    fn validate_context(&self, ctx: &ToolContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.subagent_depth == 0,
            "only the parent agent may set the shared task plan"
        );
        Ok(())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let goal_id = validate_task_plan_text("goal_id", input.goal_id, MAX_TASK_PLAN_ID_CHARS)?;
        let parsed_goal_id =
            Uuid::parse_str(&goal_id).context("set_plan goal_id must be a UUID")?;
        if let Some(expected_goal_id) = ctx.goal_id {
            anyhow::ensure!(
                parsed_goal_id == expected_goal_id,
                "set_plan must use the server-assigned goal id {expected_goal_id}"
            );
        }
        let observed_revision = ctx
            .current_task_plan
            .as_ref()
            .filter(|plan| plan.goal_id == goal_id)
            .map(|plan| plan.plan_revision)
            .unwrap_or(0);
        anyhow::ensure!(
            observed_revision == input.expected_revision,
            "stale plan revision: expected {}, current {}",
            input.expected_revision,
            observed_revision
        );
        anyhow::ensure!(
            !input.steps.is_empty(),
            "set_plan requires at least one step"
        );

        let mut steps = Vec::with_capacity(input.steps.len());
        for step in input.steps {
            let id = validate_task_plan_text("step.id", step.id, MAX_TASK_PLAN_ID_CHARS)?;
            let title =
                validate_task_plan_text("step.title", step.title, MAX_TASK_PLAN_STEP_CHARS)?;
            let dependencies = validate_task_plan_ids("step.dependencies", step.dependencies)?;
            let acceptance_criteria =
                validate_task_plan_items("step.acceptance_criteria", step.acceptance_criteria)?;
            anyhow::ensure!(
                !acceptance_criteria.is_empty(),
                "plan step {id} requires at least one acceptance criterion"
            );
            steps.push(TaskPlanStep {
                id,
                title,
                status: TaskPlanStepStatus::Pending,
                status_reason: None,
                dependencies,
                acceptance_criteria,
                evidence: Vec::new(),
            });
        }
        let plan = TaskPlan {
            plan_revision: observed_revision
                .checked_add(1)
                .context("task plan revision overflow")?,
            goal_id,
            change_reason: Some(validate_task_plan_text(
                "change_reason",
                input.change_reason,
                MAX_TASK_PLAN_CHANGE_REASON_CHARS,
            )?),
            steps,
        };
        validate_task_plan(&plan)?;
        let next_runnable_step = plan.next_runnable_step().map(|step| step.id.clone());
        let output = plan.render_for_model();
        Ok(ToolResult {
            call_id,
            output,
            content: vec![ModelContentPart::json(serde_json::to_value(&plan)?)],
            metadata: json!({
                "toolName": "set_plan",
                "taskPlan": plan,
                "nextRunnableStep": next_runnable_step,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(SetPlanTool);

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskPlanOperation {
    AppendStep,
    UpdateStep,
    RemoveStep,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AppendTaskPlanStepInput {
    id: String,
    title: String,
    status: TaskPlanStepStatus,
    #[serde(default)]
    status_reason: Option<String>,
    dependencies: Vec<String>,
    acceptance_criteria: Vec<String>,
    evidence: Vec<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateTaskPlanStepInput {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    status: Option<TaskPlanStepStatus>,
    #[serde(default)]
    status_reason: Option<String>,
    #[serde(default)]
    dependencies: Option<Vec<String>>,
    #[serde(default)]
    acceptance_criteria: Option<Vec<String>>,
    #[serde(default)]
    evidence: Option<Vec<String>>,
}

impl UpdateTaskPlanStepInput {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.status.is_none()
            && self.status_reason.is_none()
            && self.dependencies.is_none()
            && self.acceptance_criteria.is_none()
            && self.evidence.is_none()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdatePlanInput {
    /// The single atomic plan-memory mutation to apply.
    operation: TaskPlanOperation,
    /// Stable identifier for the goal whose plan is being changed.
    goal_id: String,
    /// Revision currently observed by the caller.
    expected_revision: u64,
    /// Why this mutation is necessary.
    change_reason: String,
    /// True only when every step has a terminal, explained outcome.
    #[serde(default)]
    current_scope_complete: bool,
    /// Target step id for update_step or remove_step.
    #[serde(default)]
    step_id: Option<String>,
    /// Complete step payload for append_step.
    #[serde(default)]
    step: Option<AppendTaskPlanStepInput>,
    /// Fields to replace for update_step. Omitted fields remain unchanged.
    #[serde(default)]
    updates: Option<UpdateTaskPlanStepInput>,
}

pub struct UpdatePlanTool;

#[async_trait]
impl TypedTool for UpdatePlanTool {
    type Input = UpdatePlanInput;

    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Apply one atomic append_step, update_step, or remove_step mutation to the task's external progress memory. Always send the current goal_id and expected_revision; successful changes increment the revision. next_runnable_step is an advisory dependency-aware candidate, not a mandatory scheduler decision. Deferred, blocked, and cancelled steps require a status_reason. Removal requires a concrete change_reason and is rejected while another step depends on the target."
    }

    fn validate_context(&self, ctx: &ToolContext) -> anyhow::Result<()> {
        anyhow::ensure!(
            ctx.subagent_depth == 0,
            "only the parent agent may update the shared task plan"
        );
        Ok(())
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let goal_id = validate_task_plan_text("goal_id", input.goal_id, MAX_TASK_PLAN_ID_CHARS)?;
        if let Some(expected_goal_id) = ctx.goal_id {
            anyhow::ensure!(
                goal_id == expected_goal_id.to_string(),
                "update_plan must use the server-assigned goal id {expected_goal_id}"
            );
        }
        let change_reason = validate_task_plan_text(
            "change_reason",
            input.change_reason,
            MAX_TASK_PLAN_CHANGE_REASON_CHARS,
        )?;
        let mut plan = resolve_task_plan_for_mutation(
            ctx.current_task_plan,
            &goal_id,
            input.expected_revision,
            input.operation,
        )?;

        let changed_step_id = match input.operation {
            TaskPlanOperation::AppendStep => {
                if input.step_id.is_some() || input.updates.is_some() {
                    anyhow::bail!("append_step accepts step but not step_id or updates");
                }
                let step = input
                    .step
                    .context("append_step requires a complete step payload")?;
                let step = validate_appended_task_plan_step(step)?;
                if plan.steps.iter().any(|item| item.id == step.id) {
                    anyhow::bail!("task plan already contains step id: {}", step.id);
                }
                let step_id = step.id.clone();
                plan.steps.push(step);
                step_id
            }
            TaskPlanOperation::UpdateStep => {
                if input.step.is_some() {
                    anyhow::bail!("update_step accepts step_id and updates but not step");
                }
                let step_id = validate_task_plan_text(
                    "step_id",
                    input.step_id.context("update_step requires step_id")?,
                    MAX_TASK_PLAN_ID_CHARS,
                )?;
                let updates = input.updates.context("update_step requires updates")?;
                if updates.is_empty() {
                    anyhow::bail!("update_step requires at least one changed field");
                }
                let target = plan
                    .steps
                    .iter_mut()
                    .find(|step| step.id == step_id)
                    .with_context(|| format!("task plan does not contain step id: {step_id}"))?;
                apply_task_plan_step_updates(target, updates)?;
                step_id
            }
            TaskPlanOperation::RemoveStep => {
                if input.step.is_some() || input.updates.is_some() {
                    anyhow::bail!("remove_step accepts step_id but not step or updates");
                }
                let step_id = validate_task_plan_text(
                    "step_id",
                    input.step_id.context("remove_step requires step_id")?,
                    MAX_TASK_PLAN_ID_CHARS,
                )?;
                let dependents = plan
                    .steps
                    .iter()
                    .filter(|step| {
                        step.dependencies
                            .iter()
                            .any(|dependency| dependency == &step_id)
                    })
                    .map(|step| step.id.clone())
                    .collect::<Vec<_>>();
                if !dependents.is_empty() {
                    anyhow::bail!(
                        "cannot remove step {step_id}; it is still required by: {}",
                        dependents.join(", ")
                    );
                }
                let index = plan
                    .steps
                    .iter()
                    .position(|step| step.id == step_id)
                    .with_context(|| format!("task plan does not contain step id: {step_id}"))?;
                plan.steps.remove(index);
                step_id
            }
        };

        if plan.steps.len() > MAX_TASK_PLAN_STEPS {
            anyhow::bail!("task plan may contain at most {MAX_TASK_PLAN_STEPS} steps");
        }
        validate_task_plan(&plan)?;
        if input.operation != TaskPlanOperation::RemoveStep {
            let changed_step = plan
                .steps
                .iter()
                .find(|step| step.id == changed_step_id)
                .expect("changed task plan step remains present");
            if changed_step.status == TaskPlanStepStatus::Completed
                && changed_step.acceptance_criteria.is_empty()
            {
                anyhow::bail!("completed step {changed_step_id} requires acceptance_criteria");
            }
            if changed_step.status == TaskPlanStepStatus::Completed
                && changed_step.evidence.is_empty()
            {
                anyhow::bail!("completed step {changed_step_id} requires evidence");
            }
        }

        plan.plan_revision = plan
            .plan_revision
            .checked_add(1)
            .context("task plan revision overflow")?;
        plan.goal_id = goal_id;
        plan.change_reason = Some(change_reason);
        let completed = plan
            .steps
            .iter()
            .filter(|step| step.status == TaskPlanStepStatus::Completed)
            .count();
        let resolved = plan
            .steps
            .iter()
            .filter(|step| step.status.is_resolved())
            .count();
        let verification = plan
            .steps
            .iter()
            .flat_map(|step| step.evidence.iter().cloned())
            .collect::<Vec<_>>();
        let status_reasons = plan
            .steps
            .iter()
            .filter_map(|step| step.status_reason.clone())
            .collect::<Vec<_>>();
        if input.current_scope_complete && plan.steps.is_empty() {
            anyhow::bail!("a completed current scope must contain at least one plan step");
        }
        if input.current_scope_complete && plan.has_actionable_steps() {
            anyhow::bail!("a completed current scope cannot contain pending or in_progress steps");
        }
        if input.current_scope_complete && verification.is_empty() && status_reasons.is_empty() {
            anyhow::bail!(
                "a completed current scope requires step evidence or a terminal status reason"
            );
        }
        let next_runnable_step = plan.next_runnable_step().cloned();
        let current_step_index = next_runnable_step
            .as_ref()
            .and_then(|next| plan.steps.iter().position(|step| step.id == next.id))
            .map(|index| index + 1);
        let value = serde_json::to_value(&plan)?;
        let next_runnable_value = serde_json::to_value(&next_runnable_step)?;
        Ok(ToolResult {
            call_id,
            output: format!(
                "Plan {} updated to revision {}: {resolved}/{} steps resolved.{}",
                plan.goal_id,
                plan.plan_revision,
                plan.steps.len(),
                next_runnable_step.as_ref().map_or_else(
                    || " No runnable step remains.".to_string(),
                    |step| format!(
                        " Advisory runnable candidate: {} - {}.",
                        step.id, step.title
                    )
                )
            ),
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": "update_plan",
                "taskPlan": value,
                "operation": input.operation,
                "planRevision": plan.plan_revision,
                "goalId": plan.goal_id,
                "completed": completed,
                "resolved": resolved,
                "total": plan.steps.len(),
                "allStepsComplete": !plan.steps.is_empty() && completed == plan.steps.len(),
                "allStepsResolved": !plan.steps.is_empty() && resolved == plan.steps.len(),
                "nextRunnableStep": next_runnable_value,
                "currentStepIndex": current_step_index,
                "currentScopeComplete": input.current_scope_complete,
                "verification": verification,
                "statusReasons": status_reasons,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(UpdatePlanTool);

fn resolve_task_plan_for_mutation(
    current_plan: Option<TaskPlan>,
    goal_id: &str,
    expected_revision: u64,
    operation: TaskPlanOperation,
) -> anyhow::Result<TaskPlan> {
    let Some(current_plan) = current_plan.map(TaskPlan::normalize_legacy) else {
        if operation != TaskPlanOperation::AppendStep {
            anyhow::bail!("no task plan exists; start one with append_step at expected_revision 0");
        }
        if expected_revision != 0 {
            anyhow::bail!(
                "task plan revision conflict: expected {expected_revision}, current revision is 0"
            );
        }
        return Ok(TaskPlan {
            plan_revision: 0,
            goal_id: goal_id.to_string(),
            change_reason: None,
            steps: Vec::new(),
        });
    };

    if current_plan.goal_id != goal_id {
        if operation == TaskPlanOperation::AppendStep
            && expected_revision == 0
            && !current_plan.has_actionable_steps()
        {
            return Ok(TaskPlan {
                plan_revision: 0,
                goal_id: goal_id.to_string(),
                change_reason: None,
                steps: Vec::new(),
            });
        }
        anyhow::bail!(
            "task plan goal conflict: requested {goal_id}, current goal is {} at revision {}",
            current_plan.goal_id,
            current_plan.plan_revision
        );
    }
    if expected_revision != current_plan.plan_revision {
        anyhow::bail!(
            "task plan revision conflict: expected {expected_revision}, current revision is {}",
            current_plan.plan_revision
        );
    }
    Ok(current_plan)
}

fn validate_appended_task_plan_step(
    input: AppendTaskPlanStepInput,
) -> anyhow::Result<TaskPlanStep> {
    Ok(TaskPlanStep {
        id: validate_task_plan_text("step.id", input.id, MAX_TASK_PLAN_ID_CHARS)?,
        title: validate_task_plan_text("step.title", input.title, MAX_TASK_PLAN_STEP_CHARS)?,
        status: input.status,
        status_reason: input
            .status_reason
            .map(|reason| {
                validate_task_plan_text(
                    "step.status_reason",
                    reason,
                    MAX_TASK_PLAN_STATUS_REASON_CHARS,
                )
            })
            .transpose()?,
        dependencies: validate_task_plan_ids("step.dependencies", input.dependencies)?,
        acceptance_criteria: validate_task_plan_items(
            "step.acceptance_criteria",
            input.acceptance_criteria,
        )?,
        evidence: validate_task_plan_items("step.evidence", input.evidence)?,
    })
}

fn apply_task_plan_step_updates(
    target: &mut TaskPlanStep,
    updates: UpdateTaskPlanStepInput,
) -> anyhow::Result<()> {
    if let Some(title) = updates.title {
        target.title = validate_task_plan_text("updates.title", title, MAX_TASK_PLAN_STEP_CHARS)?;
    }
    if let Some(status) = updates.status {
        target.status = status;
        if !status.requires_status_reason() {
            target.status_reason = None;
        }
    }
    if let Some(reason) = updates.status_reason {
        target.status_reason = Some(validate_task_plan_text(
            "updates.status_reason",
            reason,
            MAX_TASK_PLAN_STATUS_REASON_CHARS,
        )?);
    }
    if let Some(dependencies) = updates.dependencies {
        target.dependencies = validate_task_plan_ids("updates.dependencies", dependencies)?;
    }
    if let Some(criteria) = updates.acceptance_criteria {
        target.acceptance_criteria =
            validate_task_plan_items("updates.acceptance_criteria", criteria)?;
    }
    if let Some(evidence) = updates.evidence {
        target.evidence = validate_task_plan_items("updates.evidence", evidence)?;
    }
    Ok(())
}

fn validate_task_plan_text(field: &str, value: String, max_chars: usize) -> anyhow::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("update_plan {field} cannot be empty");
    }
    if value.chars().count() > max_chars {
        anyhow::bail!("update_plan {field} exceeds the {max_chars} character limit");
    }
    Ok(value)
}

fn validate_task_plan_items(field: &str, values: Vec<String>) -> anyhow::Result<Vec<String>> {
    if values.len() > MAX_TASK_PLAN_STEP_ITEMS {
        anyhow::bail!("update_plan {field} may contain at most {MAX_TASK_PLAN_STEP_ITEMS} items");
    }
    let mut unique = HashSet::new();
    values
        .into_iter()
        .map(|value| {
            let value = validate_task_plan_text(field, value, MAX_TASK_PLAN_STEP_CHARS)?;
            if !unique.insert(value.to_lowercase()) {
                anyhow::bail!("update_plan {field} contains a duplicate item: {value}");
            }
            Ok(value)
        })
        .collect()
}

fn validate_task_plan_ids(field: &str, values: Vec<String>) -> anyhow::Result<Vec<String>> {
    if values.len() > MAX_TASK_PLAN_STEPS {
        anyhow::bail!("update_plan {field} may contain at most {MAX_TASK_PLAN_STEPS} items");
    }
    let mut unique = HashSet::new();
    values
        .into_iter()
        .map(|value| {
            let value = validate_task_plan_text(field, value, MAX_TASK_PLAN_ID_CHARS)?;
            if !unique.insert(value.clone()) {
                anyhow::bail!("update_plan {field} contains a duplicate id: {value}");
            }
            Ok(value)
        })
        .collect()
}

fn validate_task_plan(plan: &TaskPlan) -> anyhow::Result<()> {
    let mut ids = HashSet::new();
    let mut titles = HashSet::new();
    let mut in_progress = 0usize;
    for step in &plan.steps {
        if !ids.insert(step.id.as_str()) {
            anyhow::bail!("task plan contains duplicate step id: {}", step.id);
        }
        if !titles.insert(step.title.to_lowercase()) {
            anyhow::bail!("task plan contains duplicate step title: {}", step.title);
        }
        if step.status == TaskPlanStepStatus::InProgress {
            in_progress += 1;
        }
        if step.status.requires_status_reason()
            && step.status_reason.as_deref().is_none_or(str::is_empty)
        {
            anyhow::bail!(
                "task plan step {} requires status_reason when status is {:?}",
                step.id,
                step.status
            );
        }
    }
    if in_progress > 1 {
        anyhow::bail!("task plan may contain at most one in_progress step");
    }

    for step in &plan.steps {
        for dependency in &step.dependencies {
            if dependency == &step.id {
                anyhow::bail!("task plan step {} cannot depend on itself", step.id);
            }
            let dependency_step = plan
                .steps
                .iter()
                .find(|candidate| &candidate.id == dependency)
                .with_context(|| {
                    format!(
                        "task plan step {} has unknown dependency: {dependency}",
                        step.id
                    )
                })?;
            if matches!(
                step.status,
                TaskPlanStepStatus::InProgress | TaskPlanStepStatus::Completed
            ) && dependency_step.status != TaskPlanStepStatus::Completed
            {
                anyhow::bail!(
                    "task plan step {} cannot be {:?} before dependency {dependency} is completed",
                    step.id,
                    step.status
                );
            }
        }
    }

    let mut unresolved = plan
        .steps
        .iter()
        .map(|step| {
            (
                step.id.clone(),
                step.dependencies.iter().cloned().collect::<HashSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    while !unresolved.is_empty() {
        let resolved = unresolved
            .iter()
            .filter_map(|(id, dependencies)| dependencies.is_empty().then_some(id.clone()))
            .collect::<Vec<_>>();
        if resolved.is_empty() {
            anyhow::bail!(
                "task plan contains a dependency cycle involving: {}",
                unresolved.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        for id in &resolved {
            unresolved.remove(id);
        }
        for dependencies in unresolved.values_mut() {
            for id in &resolved {
                dependencies.remove(id);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyToolInput {}

pub struct ListSkillsTool;

#[async_trait]
impl TypedTool for ListSkillsTool {
    type Input = EmptyToolInput;

    fn name(&self) -> &str {
        "list_skills"
    }

    fn description(&self) -> &str {
        "List available capability instructions (Skills) without loading their instructions."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        _input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let skills = discover_skills(Some(&ctx.workspace_root))
            .into_iter()
            .filter(|skill| {
                ctx.capability_projection.allows_skill(&skill.id)
                    && skill
                        .plugin_id
                        .as_ref()
                        .is_none_or(|plugin_id| ctx.capability_projection.allows_plugin(plugin_id))
            })
            .collect::<Vec<_>>();
        let value = serde_json::to_value(&skills)?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({ "count": skills.len() }),
        })
    }
}

impl_typed_tool!(ListSkillsTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadSkillInput {
    /// Skill ID returned by list_skills.
    id: String,
    /// Byte offset to start reading from. Defaults to 0.
    #[serde(default)]
    offset: u64,
    /// Maximum bytes to return, capped at 65536.
    #[serde(default)]
    #[schemars(range(min = 1))]
    limit: Option<u64>,
}

pub struct ReadSkillTool;

#[async_trait]
impl TypedTool for ReadSkillTool {
    type Input = ReadSkillInput;

    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Read one Skill's instructions after deciding it is relevant to the current task. Returns at most 64 KB per call; when the result reports a next offset, call again with that offset to read the rest."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let id = input.id.trim();
        anyhow::ensure!(!id.is_empty(), "read_skill id must be a non-empty string");
        anyhow::ensure!(
            ctx.capability_projection.allows_skill(id),
            "Skill is outside the active ExecutionContext projection: {id}"
        );
        let limit = input.limit.map_or(MAX_SKILL_BYTES, |value| {
            (value as usize).min(MAX_SKILL_BYTES)
        });
        // load_skill_slice resolves the opaque ID against the bounded, canonicalized Skill
        // catalog. It cannot be used as a general-purpose path read, including for user Skills
        // that intentionally live outside the thread workspace.
        let slice = load_skill_slice(Some(&ctx.workspace_root), id, input.offset, limit)?;
        if let Some(plugin_id) = slice.descriptor.plugin_id.as_ref() {
            anyhow::ensure!(
                ctx.capability_projection.allows_plugin(plugin_id),
                "Skill plugin is outside the active ExecutionContext projection: {plugin_id}"
            );
        }
        let output = slice.render_for_model();
        Ok(ToolResult {
            call_id,
            output: output.clone(),
            content: vec![ModelContentPart::text(output)],
            metadata: json!({
                "id": slice.descriptor.id,
                "name": slice.descriptor.name,
                "path": slice.descriptor.path,
                "offset": slice.offset,
                "nextOffset": slice.next_offset,
                "totalBytes": slice.total_bytes
            }),
        })
    }
}

impl_typed_tool!(ReadSkillTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateSkillToolInput {
    /// Short action-oriented lowercase hyphen-case name, at most 64 characters.
    name: String,
    /// What the Skill does and the concrete situations that should trigger it.
    description: String,
    /// Concise imperative Markdown for another agent.
    instructions: String,
    /// Installation scope. Defaults to user.
    #[serde(default)]
    scope: Option<SkillScope>,
    /// Optional human-facing title.
    #[serde(default)]
    display_name: Option<String>,
    /// Optional UI summary, at most 64 characters.
    #[serde(default)]
    short_description: Option<String>,
    /// Optional one-sentence example that mentions the Skill.
    #[serde(default)]
    default_prompt: Option<String>,
    /// Optional UTF-8 text resources.
    #[serde(default)]
    #[schemars(length(max = 24))]
    resources: Vec<SkillResourceDraft>,
}

pub struct CreateSkillTool;

#[async_trait]
impl TypedTool for CreateSkillTool {
    type Input = CreateSkillToolInput;

    fn name(&self) -> &str {
        "create_skill"
    }

    fn description(&self) -> &str {
        "Create a reusable Skill directly from the current conversation. Use when the user asks to summarize, preserve, or turn the current work into a Skill. Synthesize concise instructions and any materially useful resources from conversation context, then call this tool without a separate draft/review workflow. Default to a user Skill unless the user explicitly asks for the current project. After success, tell the user the Skill name, purpose, path, and files created."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scope = input.scope.unwrap_or(SkillScope::User);
        let name = input.name.trim().to_ascii_lowercase();
        let description = input.description.trim().to_string();
        let draft = SkillDraft {
            display_name: input
                .display_name
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| skill_display_name(&name)),
            short_description: input
                .short_description
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| truncate_chars(&description, 64)),
            default_prompt: input
                .default_prompt
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("Use ${name} to apply this reusable workflow.")),
            name,
            description,
            instructions: input.instructions,
            resources: input.resources,
        };
        let workspace_root =
            (scope == SkillScope::Workspace).then_some(ctx.workspace_root.as_path());
        let preview = preview_skill_draft(draft.clone(), scope, workspace_root)?;
        enforce_policy_decision(
            ctx.policy.inspect_write(&preview.target_path),
            ctx.approval_granted,
        )?;
        let created = create_skill_from_draft(draft, scope, workspace_root)?;
        let files = created
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        let output = format!(
            "Created Skill `{}`.\nScope: {}\nPurpose: {}\nPath: {}\nFiles:\n- {}",
            created.skill.name,
            match scope {
                SkillScope::Workspace => "workspace",
                SkillScope::User => "user",
            },
            created.skill.description,
            created.skill.path.display(),
            files.join("\n- ")
        );
        let skill = serde_json::to_value(&created.skill)?;
        Ok(ToolResult::text(
            call_id,
            output,
            json!({
                "success": true,
                "createdSkill": skill,
                "changedPath": created.skill.path,
                "changedPaths": files,
                "fileCount": created.files.len()
            }),
        ))
    }
}

impl_typed_tool!(CreateSkillTool);

fn skill_display_name(name: &str) -> String {
    name.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BrowserActionInput {
    Navigate,
    Observe,
    Screenshot,
    Click,
    Type,
    Wait,
    Download,
    Close,
}

impl BrowserActionInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::Navigate => "navigate",
            Self::Observe => "observe",
            Self::Screenshot => "screenshot",
            Self::Click => "click",
            Self::Type => "type",
            Self::Wait => "wait",
            Self::Download => "download",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BrowserWaitConditionInput {
    #[default]
    DocumentComplete,
    Selector,
    Text,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserInput {
    /// Browser action to perform.
    action: BrowserActionInput,
    /// URL for navigate or download.
    #[serde(default)]
    url: Option<String>,
    /// CSS selector for a non-mutating wait condition only.
    #[serde(default)]
    selector: Option<String>,
    /// Required for click and type; returned by observe.
    #[serde(default)]
    observation_id: Option<String>,
    /// Required for click and type; returned by observe.
    #[serde(default)]
    node_ref: Option<String>,
    /// Include a screenshot in observe; defaults to false.
    #[serde(default)]
    include_screenshot: bool,
    /// Text for type or a wait text condition.
    #[serde(default)]
    text: Option<String>,
    /// Clear an input before typing; defaults to true.
    #[serde(default = "default_true")]
    clear_first: bool,
    /// Wait condition; defaults to document_complete.
    #[serde(default)]
    condition: BrowserWaitConditionInput,
    #[serde(default)]
    #[schemars(range(min = 1, max = 120000))]
    timeout_ms: Option<u64>,
    /// Optional expected filename for a download.
    #[serde(default)]
    expected_filename: Option<String>,
}

pub struct BrowserTool;

/// Signals that a browser interaction must be completed by the user in the visible page.
/// This is distinct from an approval: the agent must stop controlling the page rather than retry
/// the same operation after a yes/no decision.
#[derive(Debug, Clone, Error)]
#[error("{reason}")]
pub struct BrowserHandoffRequired {
    pub action: String,
    pub reason: String,
    pub url: Option<String>,
}

pub fn browser_handoff_required(error: &anyhow::Error) -> Option<&BrowserHandoffRequired> {
    error.downcast_ref::<BrowserHandoffRequired>()
}

pub fn browser_handoff_for_node(
    action: &str,
    node: &crate::browser::BrowserNode,
    url: Option<String>,
) -> Option<BrowserHandoffRequired> {
    if !node.requires_user_action {
        return None;
    }
    Some(BrowserHandoffRequired {
        action: action.to_string(),
        reason: node.user_action_reason.clone().unwrap_or_else(|| {
            "This page requires you to complete the action yourself before I continue.".to_string()
        }),
        url,
    })
}

#[async_trait]
impl TypedTool for BrowserTool {
    type Input = BrowserInput;

    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Use the shared local browser. Observe before every click or type, then use the returned observationId and nodeRef. The runtime rejects stale observations; if it reports stale_observation, discard the old node reference and call observe again before retrying. Navigate and follow ordinary links normally. When a page requires a login, verification, upload, payment, publication, or irreversible submission, stop controlling the page and tell the user to complete it in the visible browser. After the user says to continue, observe the page again before interacting."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let runtime = ctx
            .browser
            .as_ref()
            .context("browser runtime is unavailable")?
            .clone();
        let thread_id = ctx.thread_id.context("browser requires a thread context")?;
        let session = BrowserSessionId::from_thread(thread_id);
        let action = input.action.as_str().to_string();
        let timeout = input
            .timeout_ms
            .map(|milliseconds| Duration::from_millis(milliseconds.clamp(1, 120_000)));
        let output = match input.action {
            BrowserActionInput::Navigate => {
                let url = required_typed_string(input.url.as_deref(), "url")?;
                let mut request = BrowserNavigateRequest::new(url);
                if let Some(wait) = request.wait.as_mut() {
                    wait.timeout = timeout;
                }
                runtime.navigate(session, request).await?
            }
            BrowserActionInput::Observe => {
                let observation = runtime
                    .observe(
                        session,
                        BrowserObserveOptions {
                            include_screenshot: input.include_screenshot,
                        },
                    )
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    None,
                ));
            }
            BrowserActionInput::Screenshot => runtime.screenshot(session).await?,
            BrowserActionInput::Click => {
                inspect_browser_interaction(&ctx)?;
                let observation_id = browser_observation_id(input.observation_id.as_deref())?;
                let node_ref = browser_node_ref(input.node_ref.as_deref())?;
                let target = runtime
                    .observation_node(session, observation_id, node_ref)
                    .await?;
                if let Some(handoff) =
                    browser_handoff_for_node(&action, &target, target.href.clone())
                {
                    return Err(handoff.into());
                }
                let receipt = runtime
                    .perform(session, observation_id, node_ref, BrowserAction::Click)
                    .await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    Some(receipt),
                ));
            }
            BrowserActionInput::Type => {
                inspect_browser_interaction(&ctx)?;
                let observation_id = browser_observation_id(input.observation_id.as_deref())?;
                let node_ref = browser_node_ref(input.node_ref.as_deref())?;
                let target = runtime
                    .observation_node(session, observation_id, node_ref)
                    .await?;
                if let Some(handoff) = browser_handoff_for_node(&action, &target, None) {
                    return Err(handoff.into());
                }
                let receipt = runtime
                    .perform(
                        session,
                        observation_id,
                        node_ref,
                        BrowserAction::Type {
                            text: required_typed_string(input.text.as_deref(), "text")?,
                            clear_first: input.clear_first,
                        },
                    )
                    .await?;
                let observation = runtime
                    .observe(session, BrowserObserveOptions::default())
                    .await?;
                return Ok(browser_observation_to_tool_result(
                    call_id,
                    action,
                    observation,
                    Some(receipt),
                ));
            }
            BrowserActionInput::Wait => {
                let condition = match input.condition {
                    BrowserWaitConditionInput::DocumentComplete => {
                        BrowserWaitCondition::DocumentComplete
                    }
                    BrowserWaitConditionInput::Selector => {
                        BrowserWaitCondition::Selector(BrowserSelector::new(
                            required_typed_string(input.selector.as_deref(), "selector")?,
                        )?)
                    }
                    BrowserWaitConditionInput::Text => BrowserWaitCondition::Text(
                        required_typed_string(input.text.as_deref(), "text")?,
                    ),
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
            BrowserActionInput::Download => {
                let url = required_typed_string(input.url.as_deref(), "url")?;
                runtime
                    .download(
                        session,
                        BrowserDownloadRequest {
                            url,
                            expected_filename: input.expected_filename,
                            timeout,
                        },
                    )
                    .await?
            }
            BrowserActionInput::Close => {
                inspect_browser_interaction(&ctx)?;
                runtime.close_session(session).await?;
                return Ok(ToolResult::text(
                    call_id,
                    "Closed this browser tab.",
                    json!({ "sessionId": session.to_string(), "action": action }),
                ));
            }
        };
        Ok(browser_output_to_tool_result(call_id, action, output))
    }
}

impl_typed_tool!(BrowserTool);

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ComputerActionInput {
    ListWindows,
    Observe,
    Click,
    Type,
    Keypress,
    Scroll,
    Drag,
    Wait,
    Close,
}

impl ComputerActionInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::ListWindows => "list_windows",
            Self::Observe => "observe",
            Self::Click => "click",
            Self::Type => "type",
            Self::Keypress => "keypress",
            Self::Scroll => "scroll",
            Self::Drag => "drag",
            Self::Wait => "wait",
            Self::Close => "close",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ComputerMouseButtonInput {
    #[default]
    Left,
    Right,
}

impl From<ComputerMouseButtonInput> for ComputerMouseButton {
    fn from(value: ComputerMouseButtonInput) -> Self {
        match value {
            ComputerMouseButtonInput::Left => Self::Left,
            ComputerMouseButtonInput::Right => Self::Right,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
enum ComputerKeyInput {
    #[serde(rename = "ENTER")]
    Enter,
    #[serde(rename = "TAB")]
    Tab,
    #[serde(rename = "ESCAPE")]
    Escape,
    #[serde(rename = "BACKSPACE")]
    Backspace,
    #[serde(rename = "LEFT")]
    Left,
    #[serde(rename = "RIGHT")]
    Right,
    #[serde(rename = "UP")]
    Up,
    #[serde(rename = "DOWN")]
    Down,
}

impl ComputerKeyInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::Enter => "ENTER",
            Self::Tab => "TAB",
            Self::Escape => "ESCAPE",
            Self::Backspace => "BACKSPACE",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Up => "UP",
            Self::Down => "DOWN",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComputerInput {
    action: ComputerActionInput,
    /// Opaque windowId returned by list_windows; required by observe.
    #[serde(default)]
    window_id: Option<String>,
    /// Required for every action after observe.
    #[serde(default)]
    observation_id: Option<String>,
    #[serde(default)]
    x: Option<u64>,
    #[serde(default)]
    y: Option<u64>,
    #[serde(default)]
    end_x: Option<u64>,
    #[serde(default)]
    end_y: Option<u64>,
    /// Mouse button for click; defaults to left.
    #[serde(default)]
    button: ComputerMouseButtonInput,
    /// Ordinary text to type. Secrets are rejected.
    #[serde(default)]
    #[schemars(length(max = 4096))]
    text: Option<String>,
    #[serde(default)]
    key: Option<ComputerKeyInput>,
    /// Vertical wheel delta for scroll.
    #[serde(default)]
    #[schemars(range(min = -12000, max = 12000))]
    delta_y: Option<i64>,
    /// Wait duration; defaults to 1000ms.
    #[serde(default)]
    #[schemars(range(min = 1, max = 30000))]
    duration_ms: Option<u64>,
}

pub struct ComputerTool;

#[async_trait]
impl TypedTool for ComputerTool {
    type Input = ComputerInput;

    fn name(&self) -> &str {
        "computer"
    }

    fn description(&self) -> &str {
        "Observe and operate a user-approved desktop window. First list windows, then observe one window. Every input action must use the latest observationId and requires explicit approval. Never use this tool for passwords, secrets, payments, publishing, deletion, UAC, or the entire desktop."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let runtime = ctx
            .computer
            .as_ref()
            .context("computer runtime is unavailable")?
            .clone();
        let thread_id = ctx
            .thread_id
            .context("computer requires a thread context")?;
        let session = ComputerSessionId::from_thread(thread_id);
        let action_name = input.action.as_str().to_string();

        match input.action {
            ComputerActionInput::ListWindows => {
                require_computer_approval(
                    &ctx,
                    "Listing desktop window titles requires approval.",
                )?;
                let windows = runtime.list_windows(session).await?;
                let value = json!({
                    "sessionId": session,
                    "windows": windows,
                    "truncated": false,
                });
                return Ok(computer_tool_result(
                    call_id,
                    action_name,
                    value,
                    None,
                    None,
                ));
            }
            ComputerActionInput::Observe => {
                require_computer_approval(
                    &ctx,
                    "Capturing a desktop window requires approval. The grant applies only to this requested window observation.",
                )?;
                let window_id = required_typed_string(input.window_id.as_deref(), "windowId")?;
                let target = runtime
                    .list_windows(session)
                    .await?
                    .into_iter()
                    .find(|target| target.window_id == window_id)
                    .context("windowId is not a visible controllable desktop window")?;
                let observation = runtime
                    .observe(session, target, ObserveOptions::default())
                    .await?;
                let value = computer_observation_summary(&observation);
                return Ok(computer_tool_result(
                    call_id,
                    action_name,
                    value,
                    Some(observation),
                    None,
                ));
            }
            ComputerActionInput::Close => {
                runtime.close_session(session).await?;
                return Ok(ToolResult::text(
                    call_id,
                    "Closed the desktop computer session for this thread.",
                    json!({ "toolName": "computer", "sessionId": session, "success": true }),
                ));
            }
            _ => {}
        }

        let action = parse_computer_action(input)?;
        if action.contains_sensitive_text() {
            anyhow::bail!("refused: input appears to contain a password, token, or other secret");
        }
        let target = runtime
            .target_for_observation(session, action.observation_id())
            .await?;
        enforce_policy_decision(
            ctx.policy.inspect_computer_action(
                &target,
                &action,
                &ComputerPolicyContext {
                    session_id: session,
                    thread_id: Some(thread_id),
                },
            ),
            ctx.approval_granted,
        )?;
        let receipt = runtime.perform(session, action).await?;
        let observation = runtime
            .observe(session, receipt.target.clone(), ObserveOptions::default())
            .await?;
        let value = json!({
            "receipt": receipt,
            "observation": computer_observation_summary(&observation),
        });
        Ok(computer_tool_result(
            call_id,
            action_name,
            value,
            Some(observation),
            None,
        ))
    }
}

impl_typed_tool!(ComputerTool);

fn require_computer_approval(ctx: &ToolContext, reason: &str) -> anyhow::Result<()> {
    if ctx.approval_granted {
        return Ok(());
    }
    Err(ApprovalRequired::new(reason).into())
}

fn parse_computer_action(input: ComputerInput) -> anyhow::Result<ComputerAction> {
    let observation_id = || required_typed_string(input.observation_id.as_deref(), "observationId");
    match input.action {
        ComputerActionInput::Click => Ok(ComputerAction::Click {
            observation_id: observation_id()?,
            x: computer_coordinate(input.x, "x")?,
            y: computer_coordinate(input.y, "y")?,
            button: input.button.into(),
        }),
        ComputerActionInput::Type => Ok(ComputerAction::Type {
            observation_id: observation_id()?,
            text: required_typed_string(input.text.as_deref(), "text")?,
        }),
        ComputerActionInput::Keypress => Ok(ComputerAction::Keypress {
            observation_id: observation_id()?,
            key: input
                .key
                .context("key is required for keypress")?
                .as_str()
                .to_string(),
        }),
        ComputerActionInput::Scroll => Ok(ComputerAction::Scroll {
            observation_id: observation_id()?,
            delta_y: input
                .delta_y
                .context("deltaY must be an integer")?
                .clamp(-12_000, 12_000) as i32,
        }),
        ComputerActionInput::Drag => Ok(ComputerAction::Drag {
            observation_id: observation_id()?,
            start_x: computer_coordinate(input.x, "x")?,
            start_y: computer_coordinate(input.y, "y")?,
            end_x: computer_coordinate(input.end_x, "endX")?,
            end_y: computer_coordinate(input.end_y, "endY")?,
        }),
        ComputerActionInput::Wait => Ok(ComputerAction::Wait {
            observation_id: observation_id()?,
            duration_ms: input.duration_ms.unwrap_or(1_000).clamp(1, 30_000),
        }),
        other => anyhow::bail!(
            "unsupported computer action for an observed window: {}",
            other.as_str()
        ),
    }
}

fn computer_coordinate(value: Option<u64>, field: &str) -> anyhow::Result<u32> {
    value
        .and_then(|value| u32::try_from(value).ok())
        .with_context(|| format!("{field} must be a non-negative integer"))
}

fn computer_observation_summary(observation: &crate::computer::ComputerObservation) -> Value {
    json!({
        "observationId": observation.observation_id,
        "sessionId": observation.session_id,
        "target": observation.target,
        "captureRect": observation.capture_rect,
        "imageWidth": observation.image_width,
        "imageHeight": observation.image_height,
        "unstable": observation.unstable,
        "capturedAt": observation.captured_at,
        "screenshotBytes": observation.screenshot.as_ref().map(|image| image.bytes.len()),
        "accessibilityTreeAvailable": observation.accessibility_tree.is_some(),
    })
}

fn computer_tool_result(
    call_id: Uuid,
    action: String,
    value: Value,
    observation: Option<crate::computer::ComputerObservation>,
    error: Option<String>,
) -> ToolResult {
    let mut content = vec![ModelContentPart::json(value.clone())];
    if let Some(image) = observation.and_then(|observation| observation.screenshot) {
        content.push(ModelContentPart::image(image.mime_type, image.bytes));
    }
    let success = error.is_none();
    let output = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    ToolResult {
        call_id,
        output,
        content,
        metadata: json!({
            "toolName": "computer",
            "action": action,
            "computer": value,
            "success": success,
            "error": error,
        }),
    }
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
        metadata: json!({ "toolName": "browser", "action": action, "url": output.url, "browser": output.metadata }),
    }
}

fn browser_observation_id(input: Option<&str>) -> anyhow::Result<BrowserObservationId> {
    serde_json::from_value(Value::String(required_typed_string(
        input,
        "observationId",
    )?))
    .context("observationId must be a browser observation ID")
}

fn browser_node_ref(input: Option<&str>) -> anyhow::Result<BrowserNodeRef> {
    serde_json::from_value(Value::String(required_typed_string(input, "nodeRef")?))
        .context("nodeRef must be a browser node reference")
}

fn browser_observation_to_tool_result(
    call_id: Uuid,
    action: String,
    observation: BrowserObservation,
    receipt: Option<BrowserActionReceipt>,
) -> ToolResult {
    let mut rendered = vec![observation.text.clone()];
    let mut content = vec![ModelContentPart::text(observation.text.clone())];
    if let Some(receipt) = &receipt {
        rendered.push(serde_json::to_string(receipt).unwrap_or_default());
        content.push(ModelContentPart::json(
            serde_json::to_value(receipt).unwrap_or(Value::Null),
        ));
    }
    let mut structured_observation = observation.clone();
    if let Some(screenshot) = structured_observation.screenshot.take() {
        rendered.push(format!(
            "[Browser screenshot: {} bytes]",
            screenshot.bytes.len()
        ));
        content.push(ModelContentPart::image(
            screenshot.mime_type,
            screenshot.bytes,
        ));
    }
    rendered.push(serde_json::to_string(&structured_observation).unwrap_or_default());
    content.push(ModelContentPart::json(
        serde_json::to_value(&structured_observation).unwrap_or(Value::Null),
    ));
    ToolResult {
        call_id,
        output: rendered.join("\n\n"),
        content,
        metadata: json!({
            "toolName": "browser",
            "action": action,
            "url": observation.url,
            "browser": {
                "observation": structured_observation,
                "receipt": receipt,
            },
        }),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum ForkTurnsInput {
    Label(String),
    Count(u64),
}

impl ForkTurnsInput {
    fn into_string(self) -> String {
        match self {
            Self::Label(value) => value,
            Self::Count(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SubagentWorkspaceModeInput {
    #[default]
    Auto,
    SharedReadOnly,
    SharedCoordinated,
    IsolatedWorktree,
}

impl SubagentWorkspaceModeInput {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::SharedReadOnly => "shared_read_only",
            Self::SharedCoordinated => "shared_coordinated",
            Self::IsolatedWorktree => "isolated_worktree",
        }
    }
}

fn default_agent_type() -> String {
    "default".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SpawnAgentInput {
    /// Stable lowercase task name used in the canonical agent path.
    #[serde(alias = "name")]
    task_name: String,
    /// Concrete initial task for the child agent.
    #[serde(alias = "input")]
    message: String,
    /// Parent history to copy: none, all, or a positive number of turns.
    #[serde(default)]
    fork_turns: Option<ForkTurnsInput>,
    /// Built-in or project agent profile name. Defaults to default.
    #[serde(default = "default_agent_type")]
    agent_type: String,
    /// Harness workspace contract.
    #[serde(default)]
    workspace_mode: SubagentWorkspaceModeInput,
}

pub struct SpawnAgentTool;

#[async_trait]
impl TypedTool for SpawnAgentTool {
    type Input = SpawnAgentInput;

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn description(&self) -> &str {
        "Create an independently running child agent. The harness can keep read-only work shared or prepare an isolated Git worktree for an independent writer; the parent remains responsible for selecting and integrating results."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
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
        let name = input.task_name.trim().to_string();
        anyhow::ensure!(!name.is_empty(), "task_name must be a non-empty string");
        let message = input.message.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "message must be a non-empty string");
        let fork_turns = input
            .fork_turns
            .map(ForkTurnsInput::into_string)
            .unwrap_or_else(|| "all".to_string());
        let agent_type = input.agent_type;
        let profiles = AgentProfileRegistry::load(&ctx.workspace_root);
        if profiles.get(&agent_type).is_none() {
            anyhow::bail!(
                "unknown agent_type `{agent_type}`; call list_agents to inspect available profiles"
            );
        }
        let profile = profiles
            .get(&agent_type)
            .context("validated agent profile disappeared")?;
        let workspace_mode = input.workspace_mode.as_str();
        let execution_contract = subagent_execution_contract(
            &ctx,
            &name,
            workspace_mode,
            profile.sandbox_mode == Some(SandboxMode::ReadOnly),
        )
        .await?;
        let initial_conversation = select_fork_conversation(&ctx.fork_conversation, &fork_turns);
        let run = scheduler.spawn_with_contract(
            SpawnSubagentRequest {
                parent_thread_id: thread_id,
                parent_turn_id,
                parent_agent_path: ctx.agent_path.clone(),
                name,
                agent_type,
                input: message,
                fork_turns,
                depth: ctx.subagent_depth.saturating_add(1),
                initial_conversation,
                initial_model_context: ctx.fork_model_context.clone(),
            },
            execution_contract.clone(),
        )?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string(&run)?,
            content: Vec::new(),
            metadata: json!({
                "toolName": "spawn_agent",
                "runId": run.id,
                "agentPath": run.agent_path,
                "status": run.status,
                "executionContract": execution_contract,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(SpawnAgentTool);

async fn subagent_execution_contract(
    ctx: &ToolContext,
    task_name: &str,
    requested: &str,
    profile_read_only: bool,
) -> anyhow::Result<SubagentExecutionContract> {
    let mode = match requested {
        "auto" if profile_read_only => SubagentWorkspaceMode::SharedReadOnly,
        "auto" => SubagentWorkspaceMode::SharedCoordinated,
        "shared_read_only" => SubagentWorkspaceMode::SharedReadOnly,
        "shared_coordinated" => SubagentWorkspaceMode::SharedCoordinated,
        "isolated_worktree" => SubagentWorkspaceMode::IsolatedWorktree,
        other => anyhow::bail!("unknown spawn_agent workspace_mode `{other}`"),
    };
    if mode != SubagentWorkspaceMode::IsolatedWorktree {
        return Ok(SubagentExecutionContract {
            workspace: SubagentWorkspaceAssignment {
                mode,
                root: Some(ctx.workspace_root.clone()),
                branch: None,
                base_commit: None,
            },
            require_structured_delivery: false,
        });
    }

    let head = ctx
        .environment
        .exec(
            ExecRequest::new("git")
                .args(["rev-parse", "--verify", "HEAD"])
                .cwd(&ctx.workspace_root),
            ctx.execution_context(Duration::from_secs(15)),
        )
        .await
        .context("isolated subagent requires a Git repository with a HEAD commit")?;
    if !head.success {
        anyhow::bail!(
            "isolated subagent could not resolve HEAD: {}",
            truncate(&String::from_utf8_lossy(&head.stderr), 2_000)
        );
    }
    let base_commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if base_commit.is_empty() {
        anyhow::bail!("isolated subagent resolved an empty HEAD commit");
    }
    let suffix = Uuid::new_v4().simple().to_string();
    let suffix = &suffix[..12];
    let branch = format!("codex/subagent/{task_name}-{suffix}");
    let root = ctx
        .workspace_root
        .join(".opentopia")
        .join("worktrees")
        .join(format!("{task_name}-{suffix}"));
    let request = isolated_subagent_worktree_request(
        ctx.workspace_root.clone(),
        root.clone(),
        branch.clone(),
        base_commit.clone(),
    )?;
    let command = format!(
        "git worktree add -b {} {} {}",
        branch,
        root.display(),
        base_commit
    );
    enforce_policy_decision(ctx.policy.inspect_command(&command), ctx.approval_granted)?;
    drop(request);

    Ok(SubagentExecutionContract {
        workspace: SubagentWorkspaceAssignment {
            mode,
            root: Some(root),
            branch: Some(branch),
            base_commit: Some(base_commit),
        },
        require_structured_delivery: true,
    })
}

fn select_fork_conversation(
    conversation: &[ModelConversationMessage],
    fork_turns: &str,
) -> Vec<ModelConversationMessage> {
    if fork_turns == "none" {
        return Vec::new();
    }
    if fork_turns == "all" {
        return conversation.to_vec();
    }
    let turns = fork_turns.parse::<usize>().unwrap_or_default();
    if turns == 0 {
        return conversation.to_vec();
    }
    let user_indexes = conversation
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            (message.role == ModelConversationRole::User).then_some(index)
        })
        .collect::<Vec<_>>();
    let start = user_indexes
        .get(user_indexes.len().saturating_sub(turns))
        .copied()
        .unwrap_or_default();
    conversation[start..].to_vec()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentTargetMessageInput {
    /// Agent UUID, canonical path, or direct child task name.
    target: String,
    /// Message or follow-up task to deliver.
    message: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AgentTargetInput {
    /// Agent UUID, canonical path, or direct child task name.
    target: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListAgentsInput {
    /// Optional canonical path prefix.
    #[serde(default)]
    path_prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentRunInput {
    /// Child run UUID.
    run_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentRunMessageInput {
    /// Child run UUID.
    run_id: String,
    /// Additional instructions.
    input: String,
}

pub struct SendAgentMessageTool;

#[async_trait]
impl TypedTool for SendAgentMessageTool {
    type Input = AgentTargetMessageInput;

    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Queue a message for any visible agent in the current task tree. This does not start a new turn when the target is idle."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = subagent_scheduler(&ctx)?;
        let target = input.target.trim();
        anyhow::ensure!(!target.is_empty(), "target must be a non-empty string");
        let message = input.message.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "message must be a non-empty string");
        let delivery = scheduler.send_message_scoped(subagent_scope(&ctx)?, target, message)?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string(&delivery)?,
            content: Vec::new(),
            metadata: json!({
                "toolName": "send_message",
                "runId": delivery.target_id,
                "agentPath": delivery.agent_path,
                "queued": delivery.queued,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(SendAgentMessageTool);

pub struct FollowupAgentTaskTool;

#[async_trait]
impl TypedTool for FollowupAgentTaskTool {
    type Input = AgentTargetMessageInput;

    fn name(&self) -> &str {
        "followup_task"
    }

    fn description(&self) -> &str {
        "Give an existing agent a follow-up task, starting a new turn when it is idle or delivering at the next boundary when it is active."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = subagent_scheduler(&ctx)?;
        let target = input.target.trim();
        anyhow::ensure!(!target.is_empty(), "target must be a non-empty string");
        let message = input.message.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "message must be a non-empty string");
        let run = scheduler.followup_task_scoped(subagent_scope(&ctx)?, target, message)?;
        Ok(agent_tool_result(
            call_id,
            "followup_task",
            &run,
            format!("Follow-up delivered to {}.", run.agent_path),
        ))
    }
}

impl_typed_tool!(FollowupAgentTaskTool);

pub struct InterruptAgentTool;

#[async_trait]
impl TypedTool for InterruptAgentTool {
    type Input = AgentTargetInput;

    fn name(&self) -> &str {
        "interrupt_agent"
    }

    fn description(&self) -> &str {
        "Interrupt an agent's current turn. The agent identity remains available for a later followup_task."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = subagent_scheduler(&ctx)?;
        let target = input.target.trim();
        anyhow::ensure!(!target.is_empty(), "target must be a non-empty string");
        let run = scheduler.resolve_scoped(subagent_scope(&ctx)?, target)?;
        scheduler.cancel_scoped(subagent_scope(&ctx)?, run.id)?;
        Ok(agent_tool_result(
            call_id,
            "interrupt_agent",
            &run,
            format!("Interrupt requested for {}.", run.agent_path),
        ))
    }
}

impl_typed_tool!(InterruptAgentTool);

pub struct ListAgentsTool;

#[async_trait]
impl TypedTool for ListAgentsTool {
    type Input = ListAgentsInput;

    fn name(&self) -> &str {
        "list_agents"
    }

    fn description(&self) -> &str {
        "List visible agents in the current root task tree with their canonical paths, profiles, status, and latest task."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = subagent_scheduler(&ctx)?;
        let runs = scheduler.list_scoped(subagent_scope(&ctx)?, input.path_prefix.as_deref());
        let run_count = runs.len();
        let profiles = AgentProfileRegistry::load(&ctx.workspace_root);
        let value = json!({
            "agents": runs,
            "availableAgentTypes": profiles.list(),
            "profileWarnings": profiles.warnings()
        });
        let output = serde_json::to_string_pretty(&value)?;
        Ok(ToolResult {
            call_id,
            output,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({ "toolName": "list_agents", "count": run_count, "success": true }),
        })
    }
}

impl_typed_tool!(ListAgentsTool);

pub struct SendAgentInputTool;

#[async_trait]
impl TypedTool for SendAgentInputTool {
    type Input = AgentRunMessageInput;

    fn name(&self) -> &str {
        "send_input"
    }

    fn description(&self) -> &str {
        "Send additional input to an active child agent."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let run_id = Uuid::parse_str(input.run_id.trim()).context("runId must be a UUID")?;
        let message = input.input.trim().to_string();
        anyhow::ensure!(!message.is_empty(), "input must be a non-empty string");
        scheduler.send_input_scoped(subagent_scope(&ctx)?, run_id, message)?;
        Ok(ToolResult {
            call_id,
            output: format!("Input delivered to subagent {run_id}."),
            content: Vec::new(),
            metadata: json!({ "toolName": "send_input", "runId": run_id, "success": true }),
        })
    }
}

impl_typed_tool!(SendAgentInputTool);

pub struct CancelAgentTool;

#[async_trait]
impl TypedTool for CancelAgentTool {
    type Input = AgentRunInput;

    fn name(&self) -> &str {
        "cancel_agent"
    }

    fn description(&self) -> &str {
        "Cancel an active child agent."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let run_id = Uuid::parse_str(input.run_id.trim()).context("runId must be a UUID")?;
        scheduler.cancel_scoped(subagent_scope(&ctx)?, run_id)?;
        Ok(ToolResult {
            call_id,
            output: format!("Cancellation requested for subagent {run_id}."),
            content: Vec::new(),
            metadata: json!({ "toolName": "cancel_agent", "runId": run_id, "success": true }),
        })
    }
}

impl_typed_tool!(CancelAgentTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WaitAgentInput {
    /// Optional agent UUID or canonical path.
    #[serde(default, alias = "runId")]
    target: Option<String>,
    /// How long to block, up to one hour.
    #[serde(default, alias = "timeoutMs")]
    #[schemars(range(min = 1, max = 3600000))]
    timeout_ms: Option<u64>,
}

pub struct WaitAgentTool;

#[async_trait]
impl TypedTool for WaitAgentTool {
    type Input = WaitAgentInput;

    fn name(&self) -> &str {
        "wait_agent"
    }

    fn description(&self) -> &str {
        "Wait for agent mailbox activity. With target/runId, wait for that agent's current turn and return its messages with the terminal result; without one, return the next mailbox or terminal update in the visible task tree."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        let timeout_ms = input
            .timeout_ms
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
            .clamp(1, MAX_WAIT_TIMEOUT_MS);
        let scope = subagent_scope(&ctx)?;
        if let Some(target) = input.target.as_deref() {
            let run = scheduler.resolve_scoped(scope.clone(), target)?;
            let run = await_cancellable(
                ctx.cancel.as_ref(),
                scheduler.wait_scoped(scope.clone(), run.id, Duration::from_millis(timeout_ms)),
            )
            .await??;
            let messages = scheduler.drain_mailbox_from_scoped(&scope, &run.agent_path);
            let message_count = messages.len();
            let value = json!({
                "agent": run,
                "messages": messages,
            });
            return Ok(ToolResult {
                call_id,
                output: serde_json::to_string_pretty(&value)?,
                content: vec![ModelContentPart::json(value)],
                metadata: json!({
                    "toolName": "wait_agent",
                    "runId": run.id,
                    "agentPath": run.agent_path,
                    "status": run.status,
                    "terminal": run.status.is_terminal(),
                    "success": run.status == SubagentRunStatus::Completed,
                    "messageCount": message_count
                }),
            });
        }
        let activity = await_cancellable(
            ctx.cancel.as_ref(),
            scheduler.wait_for_activity_scoped(scope, Duration::from_millis(timeout_ms)),
        )
        .await??;
        let update_count = activity.agents.len();
        let message_count = activity.messages.len();
        let value = serde_json::to_value(activity)?;
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: json!({
                "toolName": "wait_agent",
                "updateCount": update_count,
                "messageCount": message_count,
                "success": true
            }),
        })
    }
}

impl_typed_tool!(WaitAgentTool);

const MAX_BATCH_WAIT_AGENTS: usize = 8;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitAgentsInput {
    /// Child run UUIDs.
    #[schemars(length(min = 1, max = 8))]
    run_ids: Vec<String>,
    /// How long to block, up to one hour.
    #[serde(default)]
    #[schemars(range(min = 1, max = 3600000))]
    timeout_ms: Option<u64>,
}

pub struct WaitAgentsTool;

#[async_trait]
impl TypedTool for WaitAgentsTool {
    type Input = WaitAgentsInput;

    fn name(&self) -> &str {
        "wait_agents"
    }

    fn description(&self) -> &str {
        "Wait on several child agents at once and return as soon as the first one finishes, together with any other agent that is already done. Agents still working are reported in stillRunning and keep going; their results arrive on their own once they finish."
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let scheduler = ctx
            .subagents
            .as_ref()
            .context("subagent runtime is unavailable")?;
        if input.run_ids.is_empty() || input.run_ids.len() > MAX_BATCH_WAIT_AGENTS {
            anyhow::bail!("wait_agents requires between 1 and {MAX_BATCH_WAIT_AGENTS} run IDs");
        }
        let mut unique = HashSet::new();
        let mut run_ids = Vec::with_capacity(input.run_ids.len());
        for raw in &input.run_ids {
            let run_id = Uuid::parse_str(raw).context("wait_agents received an invalid run ID")?;
            if !unique.insert(run_id) {
                anyhow::bail!("wait_agents received duplicate run ID {run_id}");
            }
            run_ids.push(run_id);
        }

        let timeout_ms = input
            .timeout_ms
            .unwrap_or(DEFAULT_WAIT_TIMEOUT_MS)
            .clamp(1, MAX_WAIT_TIMEOUT_MS);
        let timeout = Duration::from_millis(timeout_ms);
        let scope = subagent_scope(&ctx)?;
        let mut inflight = run_ids
            .iter()
            .map(|run_id| {
                let scope = scope.clone();
                let run_id = *run_id;
                async move { (run_id, scheduler.wait_scoped(scope, run_id, timeout).await) }
            })
            .collect::<FuturesUnordered<_>>();

        // Return as soon as one agent reaches a terminal state, then harvest every
        // other agent that is already done. Blocking until the slowest child of the
        // batch finishes would withhold results the caller could act on immediately.
        let mut settled: HashMap<Uuid, SubagentRun> = HashMap::new();
        let mut wait_errors: HashMap<Uuid, String> = HashMap::new();
        await_cancellable(ctx.cancel.as_ref(), async {
            while let Some((run_id, outcome)) = inflight.next().await {
                match outcome {
                    Ok(run) => {
                        settled.insert(run_id, run);
                        break;
                    }
                    Err(error) => {
                        wait_errors.insert(run_id, error.to_string());
                    }
                }
            }
        })
        .await?;
        while let Some(Some((run_id, outcome))) = inflight.next().now_or_never() {
            match outcome {
                Ok(run) => {
                    settled.insert(run_id, run);
                }
                Err(error) => {
                    wait_errors.insert(run_id, error.to_string());
                }
            }
        }
        // Cancels the remaining waits, not the agents behind them: those keep running
        // and report back on their own.
        drop(inflight);

        let mut still_running = Vec::new();
        let runs = run_ids
            .iter()
            .map(|run_id| {
                if let Some(run) = settled.get(run_id) {
                    return json!({
                        "runId": run_id,
                        "agentPath": run.agent_path,
                        "status": run.status,
                        "result": run.result,
                        "error": run.error,
                        "terminal": run.status.is_terminal(),
                        "success": run.status == SubagentRunStatus::Completed
                    });
                }
                let current = scheduler
                    .resolve_scoped(scope.clone(), &run_id.to_string())
                    .ok();
                still_running.push(*run_id);
                json!({
                    "runId": run_id,
                    "agentPath": current.as_ref().map(|run| run.agent_path.clone()),
                    "status": current.as_ref().map(|run| run.status),
                    "terminal": false,
                    "success": false,
                    "waitError": wait_errors.get(run_id),
                })
            })
            .collect::<Vec<_>>();
        let all_terminal = runs
            .iter()
            .all(|run| run.get("terminal").and_then(Value::as_bool) == Some(true));
        let all_succeeded = runs
            .iter()
            .all(|run| run.get("success").and_then(Value::as_bool) == Some(true));
        let messages = scheduler.drain_mailbox_scoped(&scope);
        let value = json!({
            "runs": runs,
            "messages": messages,
            "allTerminal": all_terminal,
            "allSucceeded": all_succeeded,
            "stillRunning": still_running,
            "note": if still_running.is_empty() {
                "Every requested agent reached a terminal state."
            } else {
                "This call returned as soon as the first agent finished. The agents in stillRunning are unaffected and keep working; their results reach you automatically once they finish."
            },
        });
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": "wait_agents",
                "runCount": run_ids.len(),
                "settledCount": settled.len(),
                "stillRunningCount": still_running.len(),
                "allTerminal": all_terminal,
                "allSucceeded": all_succeeded,
                "success": !settled.is_empty() || run_ids.is_empty()
            }),
        })
    }
}

impl_typed_tool!(WaitAgentsTool);

/// Longest a wait tool may block.
///
/// Waiting is the cheap way to wait: a blocked tool call burns no tokens, while a
/// short cap forces the model to spend a whole round every time it polls. The cap
/// exists only so a wait cannot outlive any plausible turn, and it matches the
/// ceiling the interactive terminal already allows.
const MAX_WAIT_TIMEOUT_MS: u64 = 3_600_000;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;

/// Runs a future while staying responsive to turn cancellation.
///
/// A long wait is only acceptable if the user can still stop it.
async fn await_cancellable<F>(
    cancel: Option<&CancellationToken>,
    future: F,
) -> anyhow::Result<F::Output>
where
    F: std::future::Future,
{
    match cancel {
        Some(token) => {
            tokio::select! {
                value = future => Ok(value),
                _ = token.cancelled() => anyhow::bail!("cancelled"),
            }
        }
        None => Ok(future.await),
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
        agent_path: ctx.agent_path.clone(),
    })
}

fn subagent_scheduler(ctx: &ToolContext) -> anyhow::Result<&SubagentScheduler> {
    ctx.subagents
        .as_ref()
        .context("subagent runtime is unavailable")
}

fn agent_tool_result(
    call_id: Uuid,
    tool_name: &str,
    run: &crate::subagents::SubagentRun,
    output: String,
) -> ToolResult {
    ToolResult {
        call_id,
        output,
        content: Vec::new(),
        metadata: json!({
            "toolName": tool_name,
            "runId": run.id,
            "agentPath": run.agent_path,
            "status": run.status,
            "success": true
        }),
    }
}

fn required_typed_string(input: Option<&str>, key: &str) -> anyhow::Result<String> {
    input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .with_context(|| format!("{key} must be a non-empty string"))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListFilesInput {
    /// Directory path relative to workspace. Use `.` for the workspace root.
    path: String,
}

pub struct ListFilesTool;

#[async_trait]
impl TypedTool for ListFilesTool {
    type Input = ListFilesInput;

    fn name(&self) -> &str {
        "list_files"
    }

    fn description(&self) -> &str {
        "List direct children of a directory inside the workspace. Use `.` for the workspace root."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("dir", &input.path)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let relative = input.path.trim();
        anyhow::ensure!(
            !relative.is_empty(),
            "list_files requires a path; use `.` for the workspace root"
        );
        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let entries = tokio::task::spawn_blocking(move || list_dir_entries(&path))
            .await
            .context("list_files task failed")??;
        Ok(ToolResult {
            call_id,
            output: entries.join("\n"),
            content: Vec::new(),
            metadata: json!({ "count": entries.len() }),
        })
    }
}

impl_typed_tool!(ListFilesTool);

const READ_FILE_ARTIFACT_THRESHOLD: usize = 64_000;
const READ_FILE_WINDOW_CHARS: usize = 16_000;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FileReadInput {
    /// File path relative to workspace.
    path: String,
    /// Character offset to start reading from. Defaults to 0.
    #[serde(default)]
    offset: u64,
    /// Maximum characters to return, capped at 16000.
    #[serde(default)]
    #[schemars(range(min = 1, max = 16000))]
    limit: Option<u64>,
}

pub struct ReadFileTool;

#[async_trait]
impl TypedTool for ReadFileTool {
    type Input = FileReadInput;

    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file inside the workspace. Returns at most 16000 characters per call; when the result reports a next offset, call again with that offset to read the rest."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("file", &input.path)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let relative = input.path.trim();
        anyhow::ensure!(!relative.is_empty(), "read_file requires a path");
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

        // A window rather than a bare cap: before this, everything past the
        // first 16000 characters of a file was simply unreachable through this
        // tool, and the model could not tell that from a short file.
        let total_chars = contents.chars().count();
        let offset = input.offset as usize;
        let limit = input.limit.map_or(READ_FILE_WINDOW_CHARS, |value| {
            (value as usize).clamp(1, READ_FILE_WINDOW_CHARS)
        });
        let window: String = contents.chars().skip(offset).take(limit).collect();
        let read_to = offset.saturating_add(window.chars().count());
        let next_offset = (read_to < total_chars).then_some(read_to);

        let mut output = window;
        if let Some(next) = next_offset {
            output.push_str(&format!(
                "\n\n[characters {offset}-{} of {total_chars}; call read_file again with offset {next} for the rest]",
                read_to.saturating_sub(1)
            ));
        }
        let mut metadata = json!({
            "path": read.path.display().to_string(),
            "bytes": bytes,
            "offset": offset,
            "nextOffset": next_offset,
            "totalChars": total_chars
        });

        if bytes > READ_FILE_ARTIFACT_THRESHOLD {
            if let Some(ref store) = ctx.store {
                if let Some(thread_id) = ctx.thread_id {
                    let tool_result = ToolResult {
                        call_id,
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
            call_id,
            output,
            content: Vec::new(),
            metadata,
        })
    }
}

impl_typed_tool!(ReadFileTool);

pub struct ReadFilesTool;

const READ_FILES_MAX_ITEMS: usize = 8;
const READ_FILES_TOTAL_CHARS: usize = 64_000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadFilesInput {
    #[schemars(length(min = 1, max = 8))]
    files: Vec<FileReadInput>,
}

#[async_trait]
impl TypedTool for ReadFilesTool {
    type Input = ReadFilesInput;

    fn name(&self) -> &str {
        "read_files"
    }

    fn description(&self) -> &str {
        "Read up to 8 independent UTF-8 files concurrently. Each item supports the same character offset/limit window as read_file; the combined response is capped at 64000 characters."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let keys = input
            .files
            .iter()
            .map(|item| tool_resource_key("file", &item.path))
            .collect();
        ToolExecutionPolicy::read_only(keys)
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        if input.files.is_empty() || input.files.len() > READ_FILES_MAX_ITEMS {
            anyhow::bail!("read_files accepts between 1 and {READ_FILES_MAX_ITEMS} files per call");
        }

        // Validate every authorization boundary before starting concurrent I/O.
        // This ensures one denied path suspends the whole tool call for approval
        // instead of being hidden inside an ordinary per-file error.
        for item in &input.files {
            let relative = item.path.trim();
            anyhow::ensure!(!relative.is_empty(), "each read_files item requires a path");
            let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
            enforce_read_policy(&ctx, &logical_path)?;
        }

        let file_count = input.files.len();
        let per_file_cap = (READ_FILES_TOTAL_CHARS / file_count).min(READ_FILE_WINDOW_CHARS);
        let mut pending = FuturesUnordered::new();
        for (index, mut item) in input.files.into_iter().enumerate() {
            let requested = item
                .limit
                .map(|value| value as usize)
                .unwrap_or(per_file_cap)
                .clamp(1, per_file_cap);
            item.limit = Some(requested as u64);
            let item_ctx = ctx.clone();
            pending.push(async move {
                let path = item.path.clone();
                let result = ReadFileTool
                    .execute_typed(Uuid::new_v4(), item, item_ctx)
                    .await;
                (index, path, result)
            });
        }

        let mut ordered = vec![None; file_count];
        while let Some((index, path, result)) = pending.next().await {
            ordered[index] = Some(match result {
                Ok(result) => json!({
                    "path": path,
                    "ok": true,
                    "content": result.output,
                    "metadata": result.metadata
                }),
                Err(error) => json!({
                    "path": path,
                    "ok": false,
                    "error": error.to_string()
                }),
            });
        }
        let results = ordered.into_iter().flatten().collect::<Vec<_>>();
        let succeeded = results
            .iter()
            .filter(|result| result.get("ok").and_then(Value::as_bool) == Some(true))
            .count();
        let value = json!({ "files": results });
        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: Vec::new(),
            metadata: json!({
                "count": file_count,
                "succeeded": succeeded,
                "failed": file_count.saturating_sub(succeeded),
                "perFileLimit": per_file_cap,
                "success": succeeded == file_count
            }),
        })
    }
}

impl_typed_tool!(ReadFilesTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    /// File path relative to workspace.
    path: String,
    /// Full file contents to write.
    content: String,
}

pub struct WriteFileTool;

#[async_trait]
impl TypedTool for WriteFileTool {
    type Input = WriteFileInput;

    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write a UTF-8 text file inside the workspace."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: false,
            idempotent: true,
            parallel_safe: false,
            side_effect: ToolSideEffect::WorkspaceWrite,
            resource_keys: vec![tool_resource_key("file", &input.path)],
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let relative = input.path.trim();
        anyhow::ensure!(!relative.is_empty(), "write_file requires a path");
        let path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_policy_decision(ctx.policy.inspect_write(&path), ctx.approval_granted)?;

        let written = ctx
            .environment
            .write_file(FileWriteRequest::new(&path, input.content.into_bytes()))
            .await?;
        Ok(ToolResult {
            call_id,
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

impl_typed_tool!(WriteFileTool);

pub struct SearchTool;

const DEFAULT_SEARCH_MAX_RESULTS: usize = 100;
const SEARCH_MAX_RESULTS_LIMIT: usize = 1_000;
const SEARCH_OUTPUT_MAX_BYTES: usize = 32_000;
const SEARCH_ARTIFACT_THRESHOLD: usize = 32_000;
const FALLBACK_MAX_FILE_BYTES: u64 = 1_048_576;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchInput {
    /// Search pattern passed to rg, or substring for fallback search.
    query: String,
    /// Optional file or directory path relative to workspace.
    #[serde(default)]
    path: Option<String>,
    /// Treat the query as literal text instead of a regular expression.
    #[serde(default, alias = "fixed_strings")]
    fixed_strings: bool,
    /// Return only matches bounded by non-word characters.
    #[serde(default, alias = "word_match")]
    word_match: bool,
    /// Maximum matching lines to return.
    #[serde(default, alias = "max_results")]
    #[schemars(range(min = 1, max = 1000))]
    max_results: Option<usize>,
}

#[async_trait]
impl TypedTool for SearchTool {
    type Input = SearchInput;

    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Recursively search workspace text for candidate definitions and references with ripgrep, falling back to a literal scan. Text matches are evidence to confirm by reading code, not semantic symbol resolution."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key(
            "tree",
            input.path.as_deref().unwrap_or("."),
        )])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let query = input.query.trim();
        anyhow::ensure!(!query.is_empty(), "search requires a query");
        let relative = input
            .path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(".");
        let max_results = input
            .max_results
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_SEARCH_MAX_RESULTS)
            .min(SEARCH_MAX_RESULTS_LIMIT);
        let fixed_strings = input.fixed_strings;
        let word_match = input.word_match;

        let logical_path = normalize_workspace_path(&ctx.workspace_root, relative)?;
        enforce_read_policy(&ctx, &logical_path)?;
        let path = ctx.environment.resolve_read_path(&logical_path)?;

        let search_arg = search_command_path(relative, &path);
        let result = match run_rg_search(
            ctx.environment.as_ref(),
            &search_arg,
            query,
            max_results,
            fixed_strings,
            word_match,
        )
        .await?
        {
            Some(result) => result,
            None => {
                run_fallback_search(
                    ctx.workspace_root.clone(),
                    path.clone(),
                    ctx.policy.clone(),
                    query.to_string(),
                    max_results,
                    word_match,
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
            "fixedStrings": fixed_strings,
            "wordMatch": word_match,
            "truncated": result.truncated,
            "originalBytes": result.original_bytes,
            "outputBytes": result.output_bytes,
            "fallback": result.fallback
        });

        let mut tool_result = ToolResult {
            call_id,
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

impl_typed_tool!(SearchTool);

pub struct ShellTool;

/// Display copies of the streams kept in result metadata. They are smaller than
/// the model-facing envelope on purpose: the timeline only needs enough to show
/// the call, and the untruncated text stays in the output (or its artifact).
const SHELL_DISPLAY_STDOUT_LIMIT: usize = 16_000;
const SHELL_DISPLAY_STDERR_LIMIT: usize = 8_000;

const ARTIFACT_THRESHOLD: usize = 16_000;
/// A foreground command blocks the model for its whole runtime, so its ceiling stays
/// modest; anything longer belongs in the background, where waiting costs nothing.
const MAX_FOREGROUND_TIMEOUT_SECONDS: u64 = 1_800;
const MAX_BACKGROUND_TIMEOUT_SECONDS: u64 = 21_600;
const DEFAULT_BACKGROUND_TIMEOUT_SECONDS: u64 = 3_600;

fn background_scope(ctx: &ToolContext) -> anyhow::Result<BackgroundScope> {
    Ok(BackgroundScope {
        thread_id: ctx
            .thread_id
            .context("background commands need an owning thread")?,
        agent_path: ctx.agent_path.clone(),
    })
}

pub struct BackgroundOutputTool;

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BackgroundOutputActionInput {
    #[default]
    Read,
    List,
    Write,
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackgroundOutputInput {
    /// Operation to perform. Defaults to read.
    #[serde(default)]
    action: BackgroundOutputActionInput,
    /// Job UUID returned by shell. Required except for list.
    #[serde(default)]
    job_id: Option<String>,
    /// Input to send for write.
    #[serde(default)]
    data: Option<String>,
    /// Append a newline to data. Defaults to false.
    #[serde(default)]
    append_newline: bool,
}

#[async_trait]
impl TypedTool for BackgroundOutputTool {
    type Input = BackgroundOutputInput;

    fn name(&self) -> &str {
        "background_output"
    }

    fn description(&self) -> &str {
        "Control commands and persistent stdio sessions you started: list them, read new output, write input to an interactive session, or stop one. You do not need to poll for completion; a finished command reports itself."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let key = format!("session:{}", input.job_id.as_deref().unwrap_or("*"));
        match input.action {
            BackgroundOutputActionInput::List => {
                ToolExecutionPolicy::read_only(vec!["sessions:self".to_string()])
            }
            BackgroundOutputActionInput::Read => ToolExecutionPolicy {
                read_only: false,
                idempotent: false,
                parallel_safe: false,
                side_effect: ToolSideEffect::SessionMutation,
                resource_keys: vec![key],
            },
            BackgroundOutputActionInput::Write | BackgroundOutputActionInput::Stop => {
                ToolExecutionPolicy {
                    read_only: false,
                    idempotent: false,
                    parallel_safe: false,
                    side_effect: ToolSideEffect::SessionMutation,
                    resource_keys: vec![key],
                }
            }
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let registry = ctx
            .background
            .as_ref()
            .context("background commands are unavailable in this runtime")?;
        let scope = background_scope(&ctx)?;
        let job_id = input
            .job_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .context("jobId must be a UUID")?;

        let (value, metadata) = match input.action {
            BackgroundOutputActionInput::List => {
                let jobs = registry.list(&scope);
                let running = jobs.iter().filter(|job| !job.status.is_terminal()).count();
                (
                    json!({ "jobs": jobs, "running": running }),
                    json!({ "jobCount": jobs.len(), "running": running, "success": true }),
                )
            }
            BackgroundOutputActionInput::Stop => {
                let job_id = job_id.context("background_output stop requires jobId")?;
                registry.stop(&scope, job_id)?;
                (
                    json!({
                        "jobId": job_id,
                        "stopped": true,
                        "note": "The command was signalled to stop. Its final status arrives with the next update."
                    }),
                    json!({ "jobId": job_id, "success": true }),
                )
            }
            BackgroundOutputActionInput::Write => {
                let job_id = job_id.context("background_output write requires jobId")?;
                let mut data = input
                    .data
                    .context("background_output write requires data")?
                    .to_string();
                if input.append_newline {
                    data.push('\n');
                }
                registry
                    .write_stdin(&scope, job_id, data.as_bytes())
                    .await?;
                (
                    json!({ "jobId": job_id, "bytesWritten": data.len(), "written": true }),
                    json!({ "jobId": job_id, "bytesWritten": data.len(), "success": true }),
                )
            }
            BackgroundOutputActionInput::Read => {
                let job_id = job_id.context("background_output read requires jobId")?;
                let chunk = registry.read_output(&scope, job_id)?;
                let metadata = json!({
                    "jobId": job_id,
                    "status": chunk.job.status.as_str(),
                    "terminal": chunk.job.status.is_terminal(),
                    "exitCode": chunk.job.exit_code,
                    "success": true
                });
                (serde_json::to_value(&chunk)?, metadata)
            }
        };

        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: {
                let mut metadata = metadata;
                if let Some(object) = metadata.as_object_mut() {
                    object.insert("toolName".to_string(), json!("background_output"));
                }
                metadata
            },
        })
    }
}

impl_typed_tool!(BackgroundOutputTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShellInput {
    /// Command interpreted by the platform shell.
    command: String,
    /// Existing workspace-relative directory. Defaults to the workspace root.
    #[serde(default)]
    workdir: Option<String>,
    /// Timeout in seconds.
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Run detached and return a job id immediately.
    #[serde(default)]
    background: bool,
    /// Keep stdin open as a persistent stdio session.
    #[serde(default)]
    interactive: bool,
}

#[async_trait]
impl TypedTool for ShellTool {
    type Input = ShellInput;

    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        "Run a shell command in a workspace directory with timeout and output caps. Set background for slow commands, or interactive for a persistent stdio session that accepts input through background_output. Both return a job id immediately and report completion automatically."
    }

    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: false,
            side_effect: ToolSideEffect::Process,
            resource_keys: vec!["process:self".to_string(), "workspace:*".to_string()],
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        let command = input.command.trim();
        anyhow::ensure!(!command.is_empty(), "shell requires a command");
        enforce_policy_decision(ctx.policy.inspect_command(command), ctx.approval_granted)?;

        let interactive = input.interactive;
        let background = interactive || input.background;
        let requested_workdir = input.workdir.as_deref().unwrap_or(".");
        let logical_workdir = normalize_workspace_path(&ctx.workspace_root, requested_workdir)?;
        enforce_read_policy(&ctx, &logical_workdir)?;
        let workdir = ctx.environment.resolve_read_path(&logical_workdir)?;
        if !workdir.is_dir() {
            anyhow::bail!("shell workdir is not a directory: {}", workdir.display());
        }
        let timeout_seconds = input
            .timeout_seconds
            .unwrap_or(if background {
                DEFAULT_BACKGROUND_TIMEOUT_SECONDS
            } else {
                30
            })
            .min(if background {
                MAX_BACKGROUND_TIMEOUT_SECONDS
            } else {
                MAX_FOREGROUND_TIMEOUT_SECONDS
            });

        if interactive {
            let registry = ctx
                .background
                .as_ref()
                .context("interactive commands are unavailable in this runtime")?;
            let job = registry
                .spawn_session(
                    ctx.environment.clone(),
                    BackgroundSessionSpawnRequest {
                        scope: background_scope(&ctx)?,
                        command: command.to_string(),
                        request: ExecRequest::shell(command).cwd(&workdir),
                        context: ctx.execution_context(Duration::from_secs(timeout_seconds)),
                    },
                )
                .await?;
            let value = json!({
                "jobId": job.job_id,
                "status": job.status.as_str(),
                "command": job.command,
                "workdir": workdir.display().to_string(),
                "interactive": true,
                "transport": "stdio",
                "startedAt": job.started_at,
                "note": "The persistent stdio session is running. Use background_output write/read/stop with this job id."
            });
            return Ok(ToolResult {
                call_id,
                output: serde_json::to_string_pretty(&value)?,
                content: vec![ModelContentPart::json(value)],
                metadata: json!({
                    "toolName": "shell",
                    "background": true,
                    "interactive": true,
                    "transport": "stdio",
                    "jobId": job.job_id,
                    "workdir": workdir.display().to_string(),
                    "success": true
                }),
            });
        }

        if background {
            let registry = ctx
                .background
                .as_ref()
                .context("background commands are unavailable in this runtime")?;
            let job = registry.spawn(
                ctx.environment.clone(),
                BackgroundSpawnRequest {
                    scope: background_scope(&ctx)?,
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&workdir),
                    context: ctx.execution_context(Duration::from_secs(timeout_seconds)),
                },
            )?;
            let value = json!({
                "jobId": job.job_id,
                "status": job.status.as_str(),
                "command": job.command,
                "workdir": workdir.display().to_string(),
                "startedAt": job.started_at,
                "note": "The command is running detached. Carry on with other work: its output and exit status are delivered to you when it finishes, and background_output reads progress or stops it in the meantime."
            });
            return Ok(ToolResult {
                call_id,
                output: serde_json::to_string_pretty(&value)?,
                content: vec![ModelContentPart::json(value)],
                metadata: json!({
                    "toolName": "shell",
                    "background": true,
                    "jobId": job.job_id,
                    "workdir": workdir.display().to_string(),
                    "success": true
                }),
            });
        }

        let started_at = Instant::now();
        let output = ctx
            .environment
            .exec(
                ExecRequest::shell(command).cwd(&workdir),
                ctx.execution_context(Duration::from_secs(timeout_seconds)),
            )
            .await?;
        let duration_ms = started_at.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.success && looks_like_sandbox_denial(&stderr) {
            return Err(ApprovalRequired::new(format!(
                "Command was blocked by the sandbox: {}",
                truncate(&stderr, 2_000)
            ))
            .into());
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

        // `output` above is the model-facing envelope. The UI renders the call
        // from these structured fields instead of re-parsing that text, so a
        // terminal view can separate the command, stdout and stderr reliably.
        let mut result = ToolResult {
            call_id,
            output: combined,
            content: Vec::new(),
            metadata: json!({
                "command": command,
                "workdir": workdir.display().to_string(),
                "exitCode": output.exit_code,
                "success": output.success,
                "truncated": output.truncated,
                "durationMs": duration_ms,
                "stdout": truncate(&stdout, SHELL_DISPLAY_STDOUT_LIMIT),
                "stderr": truncate(&stderr, SHELL_DISPLAY_STDERR_LIMIT),
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

impl_typed_tool!(ShellTool);

pub struct GitDiffTool;

#[async_trait]
impl TypedTool for GitDiffTool {
    type Input = EmptyToolInput;

    fn name(&self) -> &str {
        "git_diff"
    }

    fn description(&self) -> &str {
        "Show the current git diff for the workspace."
    }

    fn execution_policy(&self, _input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec!["git:index-and-worktree".to_string()])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        _input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
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
            call_id,
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

impl_typed_tool!(GitDiffTool);

pub struct ApplyPatchTool;

/// Provider-native patch calls are normalized here instead of teaching the
/// workspace executor about any one transport. Their `diff` is commonly a bare
/// unified hunk (`@@ ...`) and therefore cannot be passed directly to `git apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativePatchOperation {
    CreateFile { path: String, diff: String },
    UpdateFile { path: String, diff: String },
    DeleteFile { path: String },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum ApplyPatchInput {
    Portable(PortablePatchInput),
    Structured(StructuredPatchInput),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PortablePatchInput {
    /// Portable unified diff patch.
    patch: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StructuredPatchInput {
    /// Structured provider-native operation.
    operation: NativePatchOperation,
}

impl NativePatchOperation {
    pub fn path(&self) -> &str {
        match self {
            Self::CreateFile { path, .. }
            | Self::UpdateFile { path, .. }
            | Self::DeleteFile { path } => path,
        }
    }
}

#[async_trait]
impl TypedTool for ApplyPatchTool {
    type Input = ApplyPatchInput;

    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply either a portable unified diff patch or one structured create_file/update_file/delete_file operation to the workspace."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let key = match input {
            ApplyPatchInput::Portable(_) => "workspace:*".to_string(),
            ApplyPatchInput::Structured(input) => tool_resource_key("file", input.operation.path()),
        };
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: false,
            side_effect: ToolSideEffect::WorkspaceWrite,
            resource_keys: vec![key],
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolContext,
    ) -> anyhow::Result<ToolResult> {
        match input {
            ApplyPatchInput::Portable(input) => {
                execute_portable_patch(call_id, &input.patch, ctx).await
            }
            ApplyPatchInput::Structured(input) => {
                execute_native_patch_operation(call_id, input.operation, ctx).await
            }
        }
    }
}

impl_typed_tool!(ApplyPatchTool);

async fn execute_portable_patch(
    call_id: Uuid,
    patch: &str,
    ctx: ToolContext,
) -> anyhow::Result<ToolResult> {
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
        return Err(ApprovalRequired::new(format!(
            "Patch was blocked by the sandbox: {}",
            truncate(&stderr, 2_000)
        ))
        .into());
    }
    if !output.success {
        anyhow::bail!(
            "git apply failed ({:?})\n{}",
            output.exit_code,
            truncate(&stderr, 12_000)
        );
    }

    Ok(ToolResult {
        call_id,
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

/// Execute one normalized native operation. This is public for transport
/// adapters that surface hosted apply-patch calls outside ordinary function
/// calling; portable callers continue to use [`ApplyPatchTool`].
pub async fn execute_native_patch_operation(
    call_id: Uuid,
    operation: NativePatchOperation,
    ctx: ToolContext,
) -> anyhow::Result<ToolResult> {
    let relative = validate_native_patch_path(operation.path())?;
    let target = normalize_workspace_path(&ctx.workspace_root, &relative)?;
    enforce_policy_decision(ctx.policy.inspect_write(&target), ctx.approval_granted)?;

    if matches!(&operation, NativePatchOperation::DeleteFile { .. }) {
        let deleted = ctx
            .environment
            .delete_file(FileDeleteRequest::new(&target))
            .await?;
        return Ok(ToolResult {
            call_id,
            output: format!("Deleted {}", deleted.path.display()),
            content: Vec::new(),
            metadata: json!({
                "success": true,
                "operation": "delete_file",
                "changedPath": deleted.path.display().to_string()
            }),
        });
    }

    let patch = native_patch_operation_to_unified_diff(&operation)?;
    let mut result = execute_portable_patch(call_id, &patch, ctx).await?;
    if let Some(metadata) = result.metadata.as_object_mut() {
        metadata.insert(
            "operation".to_string(),
            json!(match operation {
                NativePatchOperation::CreateFile { .. } => "create_file",
                NativePatchOperation::UpdateFile { .. } => "update_file",
                NativePatchOperation::DeleteFile { .. } => unreachable!(),
            }),
        );
        metadata.insert("changedPath".to_string(), json!(relative));
    }
    Ok(result)
}

pub fn native_patch_operation_to_unified_diff(
    operation: &NativePatchOperation,
) -> anyhow::Result<String> {
    let path = validate_native_patch_path(operation.path())?;
    match operation {
        NativePatchOperation::DeleteFile { .. } => {
            anyhow::bail!("delete_file is executed directly and has no supplied diff")
        }
        NativePatchOperation::CreateFile { diff, .. } => {
            let hunks = normalize_native_create_hunks(diff)?;
            Ok(format!(
                "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n{hunks}"
            ))
        }
        NativePatchOperation::UpdateFile { diff, .. } => {
            let hunks = extract_native_hunks(diff)
                .context("update_file diff must contain at least one unified @@ hunk")?;
            Ok(format!(
                "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{hunks}"
            ))
        }
    }
}

fn validate_native_patch_path(path: &str) -> anyhow::Result<String> {
    let path = path.trim();
    if path.is_empty() || path.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0')) {
        anyhow::bail!("native patch path must be a non-empty single line")
    }
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("native patch path must be workspace-relative: {path}")
    }
    Ok(path.replace('\\', "/"))
}

fn extract_native_hunks(diff: &str) -> Option<String> {
    let normalized = diff.replace("\r\n", "\n");
    let start = normalized
        .split_inclusive('\n')
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len();
            Some((current, line))
        })
        .find_map(|(offset, line)| line.starts_with("@@").then_some(offset))?;
    let mut hunks = normalized[start..].to_string();
    if !hunks.ends_with('\n') {
        hunks.push('\n');
    }
    Some(hunks)
}

fn normalize_native_create_hunks(diff: &str) -> anyhow::Result<String> {
    if let Some(hunks) = extract_native_hunks(diff) {
        return Ok(hunks);
    }
    let normalized = diff.replace("\r\n", "\n");
    let had_final_newline = normalized.ends_with('\n');
    let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
    if body.is_empty() {
        return Ok(String::new());
    }
    let lines = body.lines().collect::<Vec<_>>();
    let mut hunks = format!("@@ -0,0 +1,{} @@\n", lines.len());
    for line in lines {
        if line.starts_with('+') {
            hunks.push_str(line);
        } else {
            hunks.push('+');
            hunks.push_str(line);
        }
        hunks.push('\n');
    }
    if !had_final_newline {
        hunks.push_str("\\ No newline at end of file\n");
    }
    Ok(hunks)
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

fn tool_resource_key(kind: &str, path: &str) -> String {
    let normalized = path.trim().replace('\\', "/");
    format!(
        "{kind}:{}",
        if normalized.is_empty() {
            "*"
        } else {
            normalized.as_str()
        }
    )
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
    fixed_strings: bool,
    word_match: bool,
) -> anyhow::Result<Option<SearchRun>> {
    let mut args = vec![
        "--line-number".to_string(),
        "--column".to_string(),
        "--color".to_string(),
        "never".to_string(),
        "--no-heading".to_string(),
        "--no-messages".to_string(),
        "--max-count".to_string(),
        max_results.to_string(),
    ];
    if fixed_strings {
        args.push("--fixed-strings".to_string());
    }
    if word_match {
        args.push("--word-regexp".to_string());
    }
    args.extend([
        "--".to_string(),
        query.to_string(),
        search_path.to_string_lossy().into_owned(),
    ]);

    let output = match environment
        .exec(
            ExecRequest::new("rg").args(args),
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
    word_match: bool,
) -> anyhow::Result<SearchRun> {
    tokio::task::spawn_blocking(move || {
        let mut collector = FallbackCollector::new(max_results);
        collect_fallback_search(
            &workspace_root,
            &search_path,
            policy.as_ref(),
            &query,
            word_match,
            &mut collector,
        )?;
        let fallback = json!({
            "used": true,
            "mode": if word_match { "literal-word" } else { "substring" },
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
    word_match: bool,
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
            collect_fallback_search(
                workspace_root,
                &entry.path(),
                policy,
                query,
                word_match,
                collector,
            )?;
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
        if let Some(byte_index) = find_literal_match(line, query, word_match) {
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

fn find_literal_match(line: &str, query: &str, word_match: bool) -> Option<usize> {
    if !word_match {
        return line.find(query);
    }

    line.match_indices(query).find_map(|(byte_index, _)| {
        let before = line[..byte_index].chars().next_back();
        let after = line[byte_index + query.len()..].chars().next();
        let bounded_before = before.is_none_or(|character| !is_word_character(character));
        let bounded_after = after.is_none_or(|character| !is_word_character(character));
        (bounded_before && bounded_after).then_some(byte_index)
    })
}

fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
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

    let head_target = max_bytes / 2;
    let tail_target = max_bytes.saturating_sub(head_target);
    let mut head_end = head_target;
    while head_end > 0 && !value.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = value.len().saturating_sub(tail_target);
    while tail_start < value.len() && !value.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let omitted = tail_start.saturating_sub(head_end);
    let mut truncated = value[..head_end].to_string();
    truncated.push_str(&format!("\n\n[{omitted} bytes omitted]\n\n"));
    truncated.push_str(&value[tail_start..]);
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
    let total_chars = value.chars().count();
    if total_chars <= max_chars {
        return value.to_string();
    }

    let head_chars = max_chars / 2;
    let tail_chars = max_chars.saturating_sub(head_chars);
    let mut truncated: String = value.chars().take(head_chars).collect();
    truncated.push_str(&format!(
        "\n\n[{} characters omitted]\n\n",
        total_chars.saturating_sub(max_chars)
    ));
    truncated.extend(value.chars().skip(total_chars.saturating_sub(tail_chars)));
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
    use crate::policy::{BasicPolicyEngine, PermissionMode};
    use crate::subagents::{
        NoopSubagentObserver, SubagentExecutor, SubagentRun, SubagentSchedulerConfig,
    };
    use tokio::sync::mpsc;

    #[test]
    fn bundled_native_tools_are_not_core_tools_and_keep_their_plugin_source() {
        let core = ToolRegistry::with_core_tools();
        assert!(core.get("browser").is_none());
        assert!(core.get("computer").is_none());
        assert!(core.get("spreadsheet").is_none());

        let defaults = ToolRegistry::with_builtins();
        assert_eq!(
            defaults.source("browser"),
            Some(ToolSource::BundledPlugin {
                plugin_name: "browser-automation".to_string(),
            })
        );
        assert_eq!(
            defaults.source("computer"),
            Some(ToolSource::BundledPlugin {
                plugin_name: "computer-use".to_string(),
            })
        );
        assert_eq!(
            defaults.source("spreadsheet"),
            Some(ToolSource::BundledPlugin {
                plugin_name: "spreadsheet".to_string(),
            })
        );
        assert_eq!(defaults.source("read_file"), Some(ToolSource::Core));
    }

    #[test]
    fn builtin_registry_has_governance_metadata_and_input_schemas() {
        let catalog = ToolRegistry::with_builtins().capability_catalog();
        assert!(!catalog.is_empty());
        for tool in catalog {
            assert!(!tool.description.trim().is_empty(), "{}", tool.name);
            assert!(tool.input_schema.is_object(), "{}", tool.name);
            assert_ne!(tool.risk, ToolRiskLevel::Unknown, "{}", tool.name);
            assert!(
                !tool
                    .potential_side_effects
                    .contains(&ToolSideEffect::Unknown),
                "{}",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn skill_discovery_and_reads_honor_execution_context_projection() {
        let workspace = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace.clone(),
            PermissionMode::FullAccess,
        ));
        let mut context = ToolContext::local(workspace, policy);
        context.capability_projection = CapabilityProjection::deny_all();

        let listed = ListSkillsTool
            .execute_typed(Uuid::new_v4(), EmptyToolInput {}, context.clone())
            .await
            .unwrap();
        let catalog: Value = serde_json::from_str(&listed.output).unwrap();
        assert_eq!(catalog, serde_json::json!([]));

        let error = ReadSkillTool
            .execute_typed(
                Uuid::new_v4(),
                ReadSkillInput {
                    id: "unavailable-skill".to_string(),
                    offset: 0,
                    limit: None,
                },
                context,
            )
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("outside the active ExecutionContext projection"));
    }

    fn schema_contains_reference(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key("$ref") || object.values().any(schema_contains_reference)
            }
            Value::Array(values) => values.iter().any(schema_contains_reference),
            _ => false,
        }
    }

    #[test]
    fn every_static_builtin_uses_an_inline_derived_input_schema() {
        let registry = ToolRegistry::with_builtins();

        for (name, tool) in registry.tools.iter() {
            assert!(
                tool.has_derived_input_schema(),
                "static tool {name} bypasses the typed schema adapter"
            );
            let schema = tool.schema();
            assert!(schema.is_object(), "tool {name} schema is not an object");
            assert_eq!(
                schema.get("type").and_then(Value::as_str),
                Some("object"),
                "tool {name} must expose an object-root schema: {schema}"
            );
            assert!(
                !schema_contains_reference(&schema),
                "tool {name} schema contains a non-portable reference: {schema}"
            );
        }
    }

    #[test]
    fn derived_schema_and_typed_decoder_reject_the_same_invalid_shapes() {
        assert_eq!(
            ListFilesTool.input_error(&json!({})).as_deref(),
            Some("arguments.path is required")
        );
        assert_eq!(
            ListFilesTool
                .input_error(&json!({ "path": ".", "unexpected": true }))
                .as_deref(),
            Some("arguments.unexpected is not allowed")
        );
        assert!(SearchTool
            .input_error(&json!({
                "query": "TypedTool",
                "fixedStrings": true,
                "maxResults": 10
            }))
            .is_none());
        assert!(ApplyPatchTool
            .input_error(&json!({
                "patch": "diff --git a/a b/a",
                "operation": { "type": "delete_file", "path": "a" }
            }))
            .is_some());
    }

    #[test]
    fn list_files_requires_an_explicit_workspace_relative_path() {
        let schema = ListFilesTool.schema();

        assert_eq!(schema["required"], json!(["path"]));
        assert_eq!(schema["properties"]["path"]["type"], "string");
    }

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
                max_threads: 6,
                max_depth: 2,
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
    fn fork_conversation_selects_complete_recent_turns() {
        let message = |role, content: &str| ModelConversationMessage {
            role,
            content: content.to_string(),
            content_parts: Vec::new(),
        };
        let conversation = vec![
            message(ModelConversationRole::User, "first user"),
            message(ModelConversationRole::Assistant, "first assistant"),
            message(ModelConversationRole::User, "second user"),
            message(ModelConversationRole::Assistant, "second assistant"),
        ];

        assert!(select_fork_conversation(&conversation, "none").is_empty());
        assert_eq!(select_fork_conversation(&conversation, "all"), conversation);
        assert_eq!(
            select_fork_conversation(&conversation, "1"),
            vec![
                message(ModelConversationRole::User, "second user"),
                message(ModelConversationRole::Assistant, "second assistant"),
            ]
        );
        assert_eq!(select_fork_conversation(&conversation, "2"), conversation);
    }

    #[tokio::test]
    async fn automatic_subagent_workspace_contract_keeps_read_only_profiles_shared() {
        let context = tool_context(test_scheduler(), Uuid::new_v4(), Uuid::new_v4());
        let contract = subagent_execution_contract(&context, "research", "auto", true)
            .await
            .unwrap();
        assert_eq!(
            contract.workspace.mode,
            SubagentWorkspaceMode::SharedReadOnly
        );
        assert_eq!(contract.workspace.root, Some(context.workspace_root));
        assert!(!contract.require_structured_delivery);
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

    #[test]
    fn search_tool_exposes_exact_symbol_controls() {
        let schema = SearchTool.schema();
        let properties = schema["properties"]
            .as_object()
            .expect("search schema properties");

        assert_eq!(properties["fixedStrings"]["type"], "boolean");
        assert_eq!(properties["wordMatch"]["type"], "boolean");
        assert!(Tool::description(&SearchTool).contains("not semantic symbol resolution"));
    }

    #[test]
    fn literal_word_matching_respects_identifier_boundaries() {
        assert_eq!(find_literal_match("load();", "load", true), Some(0));
        assert_eq!(find_literal_match("service.load();", "load", true), Some(8));
        assert_eq!(find_literal_match("preload();", "load", true), None);
        assert_eq!(find_literal_match("load_more();", "load", true), None);
        assert_eq!(find_literal_match("preload();", "load", false), Some(3));
    }

    #[tokio::test]
    async fn search_tool_finds_exact_symbol_definitions_and_references_across_files() {
        let id = Uuid::new_v4();
        let workspace_root = std::env::temp_dir().join(format!("opentopia-symbol-search-{id}"));
        let source_root = workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(
            source_root.join("definition.rs"),
            "pub fn load() {}\npub fn preload() {}\n",
        )
        .unwrap();
        fs::write(
            source_root.join("caller.rs"),
            "fn run() {\n    load();\n    preload();\n}\n",
        )
        .unwrap();
        fs::write(
            workspace_root.join("literal.txt"),
            "service.load\nserviceXload\n",
        )
        .unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let mut sandbox = LocalSandboxConfig::enforce();
        sandbox.network = crate::sandbox::NetworkPolicy::Allow;
        let context =
            ToolContext::local_with_sandbox_config(workspace_root.clone(), policy, sandbox);

        let searched = SearchTool
            .execute(
                ToolCall::new(
                    "search",
                    json!({
                        "query": "load",
                        "path": "src",
                        "fixedStrings": true,
                        "wordMatch": true
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();

        assert!(
            searched.output.contains("definition.rs"),
            "unexpected search output: {:?}; metadata: {}",
            searched.output,
            searched.metadata
        );
        assert!(searched.output.contains("caller.rs"));
        assert!(!searched.output.contains("preload"));
        assert_eq!(searched.metadata["matches"], 2);
        assert_eq!(searched.metadata["fixedStrings"], true);
        assert_eq!(searched.metadata["wordMatch"], true);

        let literal = SearchTool
            .execute(
                ToolCall::new(
                    "search",
                    json!({
                        "query": "service.load",
                        "path": "literal.txt",
                        "fixedStrings": true
                    }),
                ),
                context,
            )
            .await
            .unwrap();
        assert!(literal.output.contains("service.load"));
        assert!(!literal.output.contains("serviceXload"));
        assert_eq!(literal.metadata["matches"], 1);

        fs::remove_dir_all(workspace_root).unwrap();
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

    /// Before windowing, everything past the first 16000 characters of a file
    /// was unreachable through `read_file`, and a truncated read looked the same
    /// to the model as a short file.
    #[tokio::test]
    async fn read_file_windows_reach_the_end_of_a_long_file() {
        let id = Uuid::new_v4();
        let workspace_root = std::env::temp_dir().join(format!("opentopia-read-window-{id}"));
        fs::create_dir_all(&workspace_root).unwrap();
        let contents = format!("{}TAIL", "z".repeat(READ_FILE_WINDOW_CHARS + 500));
        fs::write(workspace_root.join("long.txt"), &contents).unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::Auto,
        ));
        let context = ToolContext::local(workspace_root.clone(), policy);

        let first = ReadFileTool
            .execute(
                ToolCall::new("read_file", json!({ "path": "long.txt" })),
                context.clone(),
            )
            .await
            .unwrap();
        assert_eq!(first.metadata["offset"], 0);
        assert_eq!(first.metadata["nextOffset"], READ_FILE_WINDOW_CHARS);
        assert_eq!(first.metadata["totalChars"], contents.chars().count());
        assert!(!first.output.contains("TAIL"));
        assert!(first.output.contains("call read_file again with offset"));

        let next = first.metadata["nextOffset"].as_u64().unwrap();
        let second = ReadFileTool
            .execute(
                ToolCall::new("read_file", json!({ "path": "long.txt", "offset": next })),
                context.clone(),
            )
            .await
            .unwrap();
        assert!(second.output.contains("TAIL"), "the tail must be reachable");
        assert!(second.metadata["nextOffset"].is_null());

        let bounded = ReadFileTool
            .execute(
                ToolCall::new("read_file", json!({ "path": "long.txt", "limit": 10 })),
                context,
            )
            .await
            .unwrap();
        assert_eq!(bounded.metadata["nextOffset"], 10);

        fs::remove_dir_all(workspace_root).unwrap();
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
    fn browser_handoff_classifies_sensitive_page_controls() {
        let node: crate::browser::BrowserNode = serde_json::from_value(json!({
            "nodeRef": Uuid::new_v4().to_string(),
            "role": "button",
            "name": "Place order",
            "tagName": "button",
            "bounds": { "x": 0.0, "y": 0.0, "width": 20.0, "height": 20.0 },
            "href": null,
            "formAction": "/checkout",
            "formMethod": "post",
            "inputType": null,
            "editable": false,
            "requiresUserAction": true,
            "userActionReason": "Please review and complete the payment yourself."
        }))
        .expect("deserialize browser node");

        let handoff = browser_handoff_for_node("click", &node, None)
            .expect("sensitive control requires handoff");
        assert_eq!(handoff.action, "click");
        assert!(handoff.reason.contains("payment"));
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
    async fn model_agent_tools_isolate_root_tasks_and_allow_same_tree_peers() {
        let scheduler = test_scheduler();
        let target_thread = Uuid::new_v4();
        let target_parent = Uuid::new_v4();
        let run = scheduler
            .spawn(SpawnSubagentRequest {
                parent_thread_id: target_thread,
                parent_turn_id: target_parent,
                parent_agent_path: "/root".to_string(),
                name: "owned".to_string(),
                agent_type: "default".to_string(),
                input: "work".to_string(),
                fork_turns: "all".to_string(),
                depth: 1,
                initial_conversation: Vec::new(),
                initial_model_context: None,
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

        scheduler.cancel(run.id).unwrap();
        let peer = tool_context(scheduler.clone(), target_thread, Uuid::new_v4());
        WaitAgentTool
            .execute(
                ToolCall::new("wait_agent", json!({ "runId": run.id, "timeoutMs": 1_000 })),
                peer,
            )
            .await
            .unwrap();
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
    async fn request_user_input_builds_a_valid_plan_decision_request() {
        let workspace_root = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let mut context = ToolContext::local(workspace_root, policy);
        context.collaboration_mode = CollaborationMode::Plan;
        let result = RequestUserInputTool
            .execute(
                ToolCall::new(
                    "request_user_input",
                    json!({
                        "questions": [{
                            "id": "storage",
                            "header": "Storage",
                            "question": "Which persistence strategy should the plan use?",
                            "options": [
                                {
                                    "id": "sqlite",
                                    "label": "SQLite",
                                    "description": "Durable local state with migrations.",
                                    "recommended": true
                                },
                                {
                                    "id": "memory",
                                    "label": "In memory",
                                    "description": "Simpler but lost on restart."
                                }
                            ]
                        }]
                    }),
                ),
                context,
            )
            .await
            .expect("request user input");

        let request: UserInputRequest =
            serde_json::from_value(result.metadata["userInputRequest"].clone()).unwrap();
        assert_eq!(request.questions.len(), 1);
        assert_eq!(request.questions[0].options.len(), 2);
        assert!(request.questions[0].options[0].recommended);
        assert!(request.questions[0].allow_custom);
    }

    #[tokio::test]
    async fn set_plan_binds_to_server_goal_and_creates_a_pending_dag() {
        let workspace_root = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let goal_id = Uuid::new_v4();
        let mut context = ToolContext::local(workspace_root, policy);
        context.collaboration_mode = CollaborationMode::Plan;
        context.goal_id = Some(goal_id);
        let result = SetPlanTool
            .execute(
                ToolCall::new(
                    "set_plan",
                    json!({
                        "goal_id": goal_id,
                        "expected_revision": 0,
                        "change_reason": "Initial plan",
                        "steps": [
                            {
                                "id": "inspect",
                                "title": "Inspect the current behavior",
                                "dependencies": [],
                                "acceptance_criteria": ["Behavior is documented"]
                            },
                            {
                                "id": "implement",
                                "title": "Implement and verify the change",
                                "dependencies": ["inspect"],
                                "acceptance_criteria": ["Focused tests pass"]
                            }
                        ]
                    }),
                ),
                context,
            )
            .await
            .expect("set plan");
        let plan: TaskPlan = serde_json::from_value(result.metadata["taskPlan"].clone()).unwrap();
        assert_eq!(plan.goal_id, goal_id.to_string());
        assert_eq!(plan.plan_revision, 1);
        assert_eq!(plan.next_runnable_step().unwrap().id, "inspect");
        assert!(plan
            .steps
            .iter()
            .all(|step| step.status == TaskPlanStepStatus::Pending));
    }

    #[tokio::test]
    async fn update_plan_validates_progress_and_parent_ownership() {
        let workspace_root = std::env::current_dir().unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let result = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "append_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 0,
                        "change_reason": "Start with input inspection",
                        "step": {
                            "id": "inspect-inputs",
                            "title": "Inspect inputs",
                            "status": "in_progress",
                            "dependencies": [],
                            "acceptance_criteria": ["Inputs and constraints are understood"],
                            "evidence": []
                        }
                    }),
                ),
                ToolContext::local(workspace_root.clone(), policy.clone()),
            )
            .await
            .unwrap();
        let first_plan: TaskPlan =
            serde_json::from_value(result.metadata["taskPlan"].clone()).unwrap();
        assert_eq!(first_plan.plan_revision, 1);
        assert_eq!(first_plan.steps.len(), 1);
        assert!(first_plan.is_active());

        let mut append_context = ToolContext::local(workspace_root.clone(), policy.clone());
        append_context.current_task_plan = Some(first_plan);
        let appended = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "append_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 1,
                        "change_reason": "Add the production step after inspecting inputs",
                        "step": {
                            "id": "produce-output",
                            "title": "Produce output",
                            "status": "pending",
                            "dependencies": ["inspect-inputs"],
                            "acceptance_criteria": ["Requested output is produced"],
                            "evidence": []
                        }
                    }),
                ),
                append_context,
            )
            .await
            .unwrap();
        let plan: TaskPlan = serde_json::from_value(appended.metadata["taskPlan"].clone()).unwrap();
        assert_eq!(plan.plan_revision, 2);
        assert_eq!(plan.steps.len(), 2);
        assert!(plan.is_active());
        assert_eq!(
            appended.metadata["nextRunnableStep"]["id"],
            "inspect-inputs"
        );
        assert_eq!(appended.metadata["currentStepIndex"], 1);

        let mut stale_context = ToolContext::local(workspace_root.clone(), policy.clone());
        stale_context.current_task_plan = Some(plan.clone());
        let stale = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "update_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 1,
                        "change_reason": "This update is based on a stale snapshot",
                        "step_id": "inspect-inputs",
                        "updates": { "status": "completed", "evidence": ["Inspection recorded"] }
                    }),
                ),
                stale_context,
            )
            .await
            .unwrap_err();
        assert!(stale.to_string().contains("revision conflict"));

        let mut remove_context = ToolContext::local(workspace_root.clone(), policy.clone());
        remove_context.current_task_plan = Some(plan.clone());
        let dependent_removal = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "remove_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 2,
                        "change_reason": "The inspection step appears redundant",
                        "step_id": "inspect-inputs"
                    }),
                ),
                remove_context,
            )
            .await
            .unwrap_err();
        assert!(dependent_removal.to_string().contains("still required by"));

        let mut complete_context = ToolContext::local(workspace_root.clone(), policy.clone());
        complete_context.current_task_plan = Some(plan);
        let completed_inspection = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "update_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 2,
                        "change_reason": "Inspection finished",
                        "step_id": "inspect-inputs",
                        "updates": {
                            "status": "completed",
                            "evidence": ["Reviewed the supplied inputs"]
                        }
                    }),
                ),
                complete_context,
            )
            .await
            .unwrap();
        let completed_plan: TaskPlan =
            serde_json::from_value(completed_inspection.metadata["taskPlan"].clone()).unwrap();
        assert_eq!(completed_plan.plan_revision, 3);
        assert_eq!(
            completed_plan.steps[0].status,
            TaskPlanStepStatus::Completed
        );
        assert_eq!(
            completed_inspection.metadata["nextRunnableStep"]["id"],
            "produce-output"
        );
        assert_eq!(completed_inspection.metadata["currentStepIndex"], 2);

        let mut missing_terminal_reason_context =
            ToolContext::local(workspace_root.clone(), policy.clone());
        missing_terminal_reason_context.current_task_plan = Some(completed_plan.clone());
        let missing_terminal_reason = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "update_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 3,
                        "change_reason": "Defer the remaining output",
                        "step_id": "produce-output",
                        "updates": { "status": "deferred" }
                    }),
                ),
                missing_terminal_reason_context,
            )
            .await
            .unwrap_err();
        assert!(missing_terminal_reason
            .to_string()
            .contains("requires status_reason"));

        let mut deferred_context = ToolContext::local(workspace_root.clone(), policy.clone());
        deferred_context.current_task_plan = Some(completed_plan.clone());
        let deferred = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "update_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 3,
                        "change_reason": "The user moved output production to a later scope",
                        "current_scope_complete": true,
                        "step_id": "produce-output",
                        "updates": {
                            "status": "deferred",
                            "status_reason": "Explicitly postponed to the next requested phase"
                        }
                    }),
                ),
                deferred_context,
            )
            .await
            .unwrap();
        let deferred_plan: TaskPlan =
            serde_json::from_value(deferred.metadata["taskPlan"].clone()).unwrap();
        assert!(deferred_plan.is_active());
        assert!(!deferred_plan.has_actionable_steps());
        assert_eq!(deferred.metadata["allStepsResolved"], true);
        assert!(deferred.metadata["nextRunnableStep"].is_null());

        let mut cycle_context = ToolContext::local(workspace_root.clone(), policy.clone());
        cycle_context.current_task_plan = Some(completed_plan.clone());
        let cycle = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "update_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 3,
                        "change_reason": "Introduce an invalid reverse dependency",
                        "step_id": "inspect-inputs",
                        "updates": {
                            "status": "pending",
                            "dependencies": ["produce-output"]
                        }
                    }),
                ),
                cycle_context,
            )
            .await
            .unwrap_err();
        assert!(cycle.to_string().contains("dependency cycle"));

        let mut remove_leaf_context = ToolContext::local(workspace_root.clone(), policy.clone());
        remove_leaf_context.current_task_plan = Some(completed_plan);
        let removed = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "remove_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 3,
                        "change_reason": "The output step is explicitly deferred to another goal",
                        "current_scope_complete": true,
                        "step_id": "produce-output"
                    }),
                ),
                remove_leaf_context,
            )
            .await
            .unwrap();
        assert_eq!(removed.metadata["planRevision"], 4);
        assert_eq!(removed.metadata["currentScopeComplete"], true);

        let missing_reason = UpdatePlanTool
            .execute(
                ToolCall::new(
                    "update_plan",
                    json!({
                        "operation": "remove_step",
                        "goal_id": "deliver-output",
                        "expected_revision": 3,
                        "step_id": "produce-output"
                    }),
                ),
                ToolContext::local(workspace_root.clone(), policy.clone()),
            )
            .await
            .unwrap_err();
        assert!(missing_reason.to_string().contains("invalid arguments"));

        let mut child_context = ToolContext::local(workspace_root, policy);
        child_context.subagent_depth = 1;
        let denied = UpdatePlanTool
            .execute(ToolCall::new("update_plan", json!({})), child_context)
            .await
            .unwrap_err();
        assert!(denied.to_string().contains("only the parent agent"));
    }

    #[tokio::test]
    async fn wait_agents_collects_parallel_child_results() {
        let scheduler = SubagentScheduler::new(
            SubagentSchedulerConfig {
                max_concurrency_per_parent: 2,
                max_threads: 6,
                max_depth: 2,
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
                parent_agent_path: "/root".to_string(),
                name: "first".to_string(),
                agent_type: "default".to_string(),
                input: "alpha".to_string(),
                fork_turns: "all".to_string(),
                depth: 1,
                initial_conversation: Vec::new(),
                initial_model_context: None,
            })
            .unwrap();
        let second = scheduler
            .spawn(SpawnSubagentRequest {
                parent_thread_id: thread_id,
                parent_turn_id,
                parent_agent_path: "/root".to_string(),
                name: "second".to_string(),
                agent_type: "default".to_string(),
                input: "beta".to_string(),
                fork_turns: "all".to_string(),
                depth: 1,
                initial_conversation: Vec::new(),
                initial_model_context: None,
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

    #[test]
    fn truncation_preserves_diagnostic_head_and_tail() {
        let value = format!("HEAD{}TAIL", "x".repeat(100));
        let truncated = truncate(&value, 20);
        assert!(truncated.starts_with("HEAD"));
        assert!(truncated.ends_with("TAIL"));
        assert!(truncated.contains("characters omitted"));

        let (bytes, was_truncated) = truncate_bytes(&value, 20);
        assert!(was_truncated);
        assert!(bytes.starts_with("HEAD"));
        assert!(bytes.ends_with("TAIL"));
        assert!(bytes.contains("bytes omitted"));
    }

    #[tokio::test]
    async fn read_files_reads_multiple_windows_in_one_call() {
        let workspace_root =
            std::env::temp_dir().join(format!("opentopia-read-files-{}", Uuid::new_v4()));
        fs::create_dir_all(&workspace_root).unwrap();
        fs::write(workspace_root.join("a.txt"), "alpha").unwrap();
        fs::write(workspace_root.join("b.txt"), "bravo").unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let context = ToolContext::local(workspace_root.clone(), policy);

        let result = ReadFilesTool
            .execute(
                ToolCall::new(
                    "read_files",
                    json!({ "files": [{ "path": "a.txt" }, { "path": "b.txt" }] }),
                ),
                context,
            )
            .await
            .unwrap();
        assert!(result.output.contains("alpha"));
        assert!(result.output.contains("bravo"));
        assert_eq!(result.metadata["succeeded"], 2);
        fs::remove_dir_all(workspace_root).unwrap();
    }

    #[tokio::test]
    async fn shell_honors_workspace_relative_workdir() {
        let workspace_root =
            std::env::temp_dir().join(format!("opentopia-shell-workdir-{}", Uuid::new_v4()));
        fs::create_dir_all(workspace_root.join("nested")).unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let context = ToolContext::local_with_sandbox_config(
            workspace_root.clone(),
            policy,
            LocalSandboxConfig::danger_full_access(),
        );
        let command = if cfg!(windows) {
            "(Get-Location).Path"
        } else {
            "pwd"
        };
        let result = ShellTool
            .execute(
                ToolCall::new("shell", json!({ "command": command, "workdir": "nested" })),
                context,
            )
            .await
            .unwrap();
        assert!(result.output.contains("nested"));
        assert!(result.metadata["workdir"]
            .as_str()
            .is_some_and(|path| path.ends_with("nested")));
        fs::remove_dir_all(workspace_root).unwrap();
    }

    #[test]
    fn tool_execution_policy_marks_observations_as_parallel_safe() {
        let registry = ToolRegistry::with_core_tools();
        let read = ToolCall::new("read_file", json!({ "path": "src/lib.rs" }));
        let policy = registry.execution_policy("read_file", &read).unwrap();
        assert!(policy.read_only);
        assert!(policy.idempotent);
        assert!(policy.parallel_safe);
        assert_eq!(policy.side_effect, ToolSideEffect::None);
        assert_eq!(policy.resource_keys, vec!["file:src/lib.rs"]);

        let shell = ToolCall::new("shell", json!({ "command": "git status" }));
        let policy = registry.execution_policy("shell", &shell).unwrap();
        assert!(!policy.read_only);
        assert!(!policy.parallel_safe);
        assert_eq!(policy.side_effect, ToolSideEffect::Process);
    }

    #[test]
    fn plan_tools_describe_memory_and_evidence_without_mandating_a_scheduler() {
        assert!(Tool::description(&SetPlanTool).contains("external memory"));
        assert!(Tool::description(&UpdatePlanTool).contains("advisory"));
        assert!(!Tool::description(&UpdatePlanTool).contains("one step at a time"));
        assert!(Tool::description(&CompleteTaskTool).contains("verification evidence"));
    }

    #[tokio::test]
    async fn native_patch_operations_create_update_and_delete_one_target() {
        let workspace_root =
            std::env::temp_dir().join(format!("opentopia-native-patch-{}", Uuid::new_v4()));
        fs::create_dir_all(&workspace_root).unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace_root.clone(),
            PermissionMode::FullAccess,
        ));
        let context = ToolContext::local_with_sandbox_config(
            workspace_root.clone(),
            policy,
            LocalSandboxConfig::danger_full_access(),
        );

        ApplyPatchTool
            .execute(
                ToolCall::new(
                    "apply_patch",
                    json!({
                        "operation": {
                            "type": "create_file",
                            "path": "notes.txt",
                            "diff": "@@ -0,0 +1,2 @@\n+hello\n+world\n"
                        }
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(workspace_root.join("notes.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "hello\nworld\n"
        );

        execute_native_patch_operation(
            Uuid::new_v4(),
            NativePatchOperation::UpdateFile {
                path: "notes.txt".to_string(),
                diff: "@@ -1,2 +1,2 @@\n hello\n-world\n+earth\n".to_string(),
            },
            context.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(workspace_root.join("notes.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "hello\nearth\n"
        );

        execute_native_patch_operation(
            Uuid::new_v4(),
            NativePatchOperation::DeleteFile {
                path: "notes.txt".to_string(),
            },
            context,
        )
        .await
        .unwrap();
        assert!(!workspace_root.join("notes.txt").exists());
        fs::remove_dir_all(workspace_root).unwrap();
    }

    #[test]
    fn native_patch_rejects_path_injection_and_retargets_full_diffs() {
        let error = native_patch_operation_to_unified_diff(&NativePatchOperation::UpdateFile {
            path: "../escape.txt".to_string(),
            diff: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
        })
        .unwrap_err();
        assert!(error.to_string().contains("workspace-relative"));

        let patch = native_patch_operation_to_unified_diff(&NativePatchOperation::UpdateFile {
            path: "safe.txt".to_string(),
            diff: "--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
        })
        .unwrap();
        assert!(patch.contains("--- a/safe.txt"));
        assert!(!patch.contains("other.txt"));
    }
}
