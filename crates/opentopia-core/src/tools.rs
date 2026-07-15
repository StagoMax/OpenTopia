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
use crate::model::{ApprovalStatus, ModelContentPart, ToolCall, ToolResult};
use crate::policy::{PolicyDecision, PolicyEngine, ToolPermissionDescriptor};
use crate::sandbox::LocalSandboxConfig;
use crate::skills::{discover_skills, load_selected_skills};
use crate::store::SessionStore;
use crate::subagents::{SpawnSubagentRequest, SubagentScheduler, SubagentScope};
use anyhow::Context;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
        tools.insert("list_skills".to_string(), Arc::new(ListSkillsTool));
        tools.insert("read_skill".to_string(), Arc::new(ReadSkillTool));
        tools.insert("browser".to_string(), Arc::new(BrowserTool));
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
        let descriptor = discover_skills(Some(&ctx.workspace_root))
            .into_iter()
            .find(|skill| skill.id == id)
            .context("Skill is unavailable")?;
        match ctx.policy.inspect_read(&descriptor.path) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }
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
                runtime
                    .navigate(session, BrowserNavigateRequest::new(url))
                    .await?
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
    match ctx.policy.inspect_network(&host) {
        PolicyDecision::Allow => {}
        PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
        PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
    }

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
    match ctx.policy.inspect_network("browser-interaction") {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
        PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
    }
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
                "success": run.status.is_terminal()
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
        let path = normalize_workspace_path(&ctx.workspace_root, relative);
        match ctx.policy.inspect_read(&path) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

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
        let path = normalize_workspace_path(&ctx.workspace_root, relative);
        match ctx.policy.inspect_read(&path) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

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
        let path = normalize_workspace_path(&ctx.workspace_root, relative);
        match ctx.policy.inspect_write(&path) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

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

        let path = normalize_workspace_path(&ctx.workspace_root, relative);
        match ctx.policy.inspect_read(&path) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

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
        match ctx.policy.inspect_command(command) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

        let timeout_seconds = call
            .input
            .get("timeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(30)
            .min(300);

        let output = ctx
            .environment
            .exec(
                ExecRequest::shell(command),
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
                "success": output.success
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
                "success": output.success
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

        match ctx
            .policy
            .inspect_command("git apply --whitespace=nowarn -")
        {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

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
                "bytes": result.bytes
            }),
        })
    }
}

fn normalize_workspace_path(workspace_root: &Path, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        workspace_root.join(candidate)
    }
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
        json!({ "used": false }),
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
        match ctx.policy.inspect_mcp_tool_call(&permission) {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

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

    fn test_scheduler() -> SubagentScheduler {
        SubagentScheduler::new(
            SubagentSchedulerConfig {
                max_concurrency_per_parent: 1,
                max_depth: 2,
                timeout: Duration::from_secs(10),
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
}
