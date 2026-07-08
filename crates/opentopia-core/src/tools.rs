use crate::model::{ToolCall, ToolResult};
use crate::policy::{PolicyDecision, PolicyEngine};
use anyhow::Context;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::io::AsyncWriteExt;
use tokio::time::timeout;

#[derive(Clone)]
pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub policy: Arc<dyn PolicyEngine>,
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

        let contents = tokio::fs::read_to_string(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        Ok(ToolResult {
            call_id: call.id,
            output: truncate(&contents, 16_000),
            metadata: json!({
                "path": path.display().to_string(),
                "bytes": contents.len()
            }),
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

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, content).await?;
        Ok(ToolResult {
            call_id: call.id,
            output: format!("Wrote {} bytes to {}", content.len(), path.display()),
            metadata: json!({
                "changedPath": path.display().to_string(),
                "bytes": content.len()
            }),
        })
    }
}

pub struct ShellTool;

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

        let mut process = platform_shell_command(command);
        process
            .current_dir(&ctx.workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = timeout(Duration::from_secs(timeout_seconds), process.output())
            .await
            .context("command timed out")??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!(
            "$ {}\n\n[stdout]\n{}\n\n[stderr]\n{}",
            command,
            truncate(&stdout, 24_000),
            truncate(&stderr, 12_000)
        );

        Ok(ToolResult {
            call_id: call.id,
            output: combined,
            metadata: json!({
                "exitCode": output.status.code(),
                "success": output.status.success()
            }),
        })
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
        let mut process = Command::new("git");
        process
            .arg("diff")
            .arg("--")
            .current_dir(&ctx.workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = timeout(Duration::from_secs(20), process.output())
            .await
            .context("git diff timed out")??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let text = if stdout.trim().is_empty() {
            format!("[stdout]\n(no diff)\n\n[stderr]\n{}", truncate(&stderr, 8_000))
        } else {
            truncate(&stdout, 32_000)
        };
        Ok(ToolResult {
            call_id: call.id,
            output: text,
            metadata: json!({
                "exitCode": output.status.code(),
                "success": output.status.success()
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

        match ctx.policy.inspect_command("git apply --whitespace=nowarn -") {
            PolicyDecision::Allow => {}
            PolicyDecision::Deny { reason } => anyhow::bail!("denied: {reason}"),
            PolicyDecision::Ask { reason } => anyhow::bail!("approval required: {reason}"),
        }

        let mut process = Command::new("git");
        process
            .arg("apply")
            .arg("--whitespace=nowarn")
            .arg("-")
            .current_dir(&ctx.workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = process.spawn().context("failed to spawn git apply")?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(patch.as_bytes()).await?;
        }
        let output = timeout(Duration::from_secs(30), child.wait_with_output())
            .await
            .context("git apply timed out")??;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            anyhow::bail!(
                "git apply failed ({:?})\n{}",
                output.status.code(),
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
                "bytes": patch.len()
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

fn platform_shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut process = Command::new("powershell.exe");
        process
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-Command")
            .arg(command);
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-lc").arg(command);
        process
    }
}

fn list_dir_entries(path: &Path) -> anyhow::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("failed to list {}", path.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let marker = if file_type.is_dir() { "/" } else { "" };
        entries.push(format!("{}{}", entry.file_name().to_string_lossy(), marker));
    }
    entries.sort();
    Ok(entries)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated: String = value.chars().take(max_chars).collect();
    truncated.push_str("\n\n[output truncated]");
    truncated
}
