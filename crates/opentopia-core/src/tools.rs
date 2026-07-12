use crate::execution::{
    ExecRequest, ExecutionContext, ExecutionEnvironment, FileReadRequest, FileWriteRequest,
    LocalExecutionEnvironment,
};
use crate::mcp::{McpCallResult, McpToolDescriptor};
use crate::mcp_host::McpExtensionHost;
use crate::model::{ToolCall, ToolResult};
use crate::policy::{PolicyDecision, PolicyEngine, ToolPermissionDescriptor};
use crate::sandbox::LocalSandboxConfig;
use crate::store::SessionStore;
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
}

impl ToolContext {
    pub fn local(workspace_root: PathBuf, policy: Arc<dyn PolicyEngine>) -> Self {
        let environment = Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            workspace_root.clone(),
            LocalSandboxConfig::from_env(),
        ));
        Self {
            workspace_root,
            policy,
            environment,
            store: None,
            thread_id: None,
            cancel: None,
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
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
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

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn description(&self) -> &'static str {
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
            metadata: json!({ "count": entries.len() }),
        })
    }
}

pub struct ReadFileTool;

const READ_FILE_ARTIFACT_THRESHOLD: usize = 64_000;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
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
            metadata,
        })
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
        "search"
    }

    fn description(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
        "git_diff"
    }

    fn description(&self) -> &'static str {
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
    fn name(&self) -> &'static str {
        "apply_patch"
    }

    fn description(&self) -> &'static str {
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
    leaked_name: &'static str,
    leaked_description: &'static str,
}

impl McpToolWrapper {
    pub fn new(host: McpExtensionHost, descriptor: McpToolDescriptor) -> Self {
        let leaked_name = Box::leak(descriptor.public_name.clone().into_boxed_str());
        let leaked_description = Box::leak(
            descriptor
                .description
                .clone()
                .unwrap_or_default()
                .into_boxed_str(),
        );
        Self {
            host,
            descriptor,
            leaked_name,
            leaked_description,
        }
    }

    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &'static str {
        self.leaked_name
    }

    fn description(&self) -> &'static str {
        self.leaked_description
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

        Ok(ToolResult {
            call_id: call.id,
            output: result.output,
            metadata: json!({
                "isError": result.is_error,
                "publicName": result.public_name,
                "toolName": result.tool_name,
                "serverId": result.server_id,
            }),
        })
    }
}
