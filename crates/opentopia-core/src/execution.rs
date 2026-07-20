use crate::policy::ApprovalRequired;
use crate::sandbox::{
    build_local_sandbox_command, is_protected_metadata_path, sandbox_permission_profile,
    ExecutionEnvironmentKind, LocalSandboxConfig, NetworkPolicy, OsSandboxPlatform,
    SandboxCommandStatus, SandboxMode,
};
use anyhow::Context;
use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const SENSITIVE_CHILD_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENTOPIA_API_KEY",
    "OPENTOPIA_API_TOKEN",
    "CREDIT_REVIEW_LLM_API_KEY",
];

#[derive(Debug, Clone)]
pub struct ResourceLimit {
    pub max_cpu_time: Option<Duration>,
    pub max_memory_bytes: Option<u64>,
    pub max_output_bytes: Option<usize>,
}

impl Default for ResourceLimit {
    fn default() -> Self {
        Self {
            max_cpu_time: None,
            max_memory_bytes: None,
            max_output_bytes: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub timeout: Duration,
    pub cancel: Option<CancellationToken>,
    pub resource_limits: ResourceLimit,
}

impl ExecutionContext {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            cancel: None,
            resource_limits: ResourceLimit::default(),
        }
    }

    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn with_resource_limits(mut self, limits: ResourceLimit) -> Self {
        self.resource_limits = limits;
        self
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            cancel: None,
            resource_limits: ResourceLimit::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<Vec<u8>>,
    pub clear_env: bool,
    pub env: HashMap<OsString, OsString>,
}

impl ExecRequest {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            stdin: None,
            clear_env: false,
            env: HashMap::new(),
        }
    }

    pub fn shell(command: impl Into<String>) -> Self {
        let command = command.into();
        if cfg!(windows) {
            Self::new("powershell.exe")
                .arg("-NoProfile")
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-Command")
                .arg(command)
        } else {
            Self::new("sh").arg("-lc").arg(command)
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    pub fn env_clear(mut self) -> Self {
        self.clear_env = true;
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn envs<K, V>(mut self, variables: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.extend(
            variables
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }
}

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub truncated: bool,
    pub sandbox: Option<ExecutionSandboxMetadata>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSandboxMetadata {
    pub status: SandboxCommandStatus,
    pub permission_profile: String,
    pub sandbox_mode: SandboxMode,
    pub network: NetworkPolicy,
}

#[derive(Debug, Clone)]
pub struct FileReadRequest {
    pub path: PathBuf,
    pub max_bytes: Option<u64>,
}

impl FileReadRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            max_bytes: None,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }
}

#[derive(Debug, Clone)]
pub struct FileReadResult {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct FileWriteRequest {
    pub path: PathBuf,
    pub contents: Vec<u8>,
    pub create_parent_dirs: bool,
}

impl FileWriteRequest {
    pub fn new(path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            contents: contents.into(),
            create_parent_dirs: true,
        }
    }

    pub fn create_parent_dirs(mut self, create_parent_dirs: bool) -> Self {
        self.create_parent_dirs = create_parent_dirs;
        self
    }
}

#[derive(Debug, Clone)]
pub struct WriteResult {
    pub path: PathBuf,
    pub bytes_written: usize,
}

#[derive(Debug, Clone)]
pub struct PatchResult {
    pub exec: ExecResult,
    pub bytes: usize,
}

#[async_trait]
pub trait StdioSession: Send + Sync {
    async fn write_stdin(&self, data: &[u8]) -> anyhow::Result<()>;
    async fn read_stdout(&self) -> anyhow::Result<Vec<u8>>;
    async fn read_stderr(&self) -> anyhow::Result<Vec<u8>>;
    async fn close(&self) -> anyhow::Result<ExecResult>;
    async fn kill(&self) -> anyhow::Result<()> {
        Ok(())
    }
    fn request_id(&self) -> Option<&str> {
        None
    }
}

#[async_trait]
pub trait ExecutionEnvironment: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> ExecutionEnvironmentKind;
    fn workspace_root(&self) -> &Path;

    fn resolve_read_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        if path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            anyhow::bail!("workspace path cannot contain '..': {}", path.display());
        }
        let workspace_root = self.workspace_root().canonicalize().with_context(|| {
            format!(
                "workspace root does not exist: {}",
                self.workspace_root().display()
            )
        })?;
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
        let resolved = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {}", candidate.display()))?;
        if !resolved.starts_with(&workspace_root) {
            anyhow::bail!(
                "path is outside the workspace and no readable root authorized it: {}",
                path.display()
            );
        }
        Ok(resolved)
    }

    async fn exec(
        &self,
        request: ExecRequest,
        context: ExecutionContext,
    ) -> anyhow::Result<ExecResult>;

    async fn spawn_stdio(
        &self,
        request: ExecRequest,
        context: ExecutionContext,
    ) -> anyhow::Result<Box<dyn StdioSession>>;

    async fn read_file(&self, request: FileReadRequest) -> anyhow::Result<FileReadResult>;
    async fn write_file(&self, request: FileWriteRequest) -> anyhow::Result<WriteResult>;

    async fn cancel(&self, request_id: &str) -> anyhow::Result<()>;

    async fn apply_patch(
        &self,
        patch: &str,
        context: ExecutionContext,
    ) -> anyhow::Result<PatchResult> {
        let exec = self
            .exec(
                ExecRequest::new("git")
                    .args(["apply", "--whitespace=nowarn", "-"])
                    .stdin(patch.as_bytes().to_vec()),
                context,
            )
            .await?;
        Ok(PatchResult {
            exec,
            bytes: patch.len(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct LocalExecutionEnvironment {
    id: String,
    workspace_root: PathBuf,
    sandbox_config: LocalSandboxConfig,
    running: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl LocalExecutionEnvironment {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            id: "local".to_string(),
            workspace_root: workspace_root.into(),
            sandbox_config: LocalSandboxConfig::default(),
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_id(id: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            workspace_root: workspace_root.into(),
            sandbox_config: LocalSandboxConfig::default(),
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_sandbox_config(
        workspace_root: impl Into<PathBuf>,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        Self {
            id: "local".to_string(),
            workspace_root: workspace_root.into(),
            sandbox_config,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_id_and_sandbox_config(
        id: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        Self {
            id: id.into(),
            workspace_root: workspace_root.into(),
            sandbox_config,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn sandbox_config(&self) -> &LocalSandboxConfig {
        &self.sandbox_config
    }

    fn workspace_root_canonical(&self) -> anyhow::Result<PathBuf> {
        self.workspace_root.canonicalize().with_context(|| {
            format!(
                "workspace root does not exist: {}",
                self.workspace_root.display()
            )
        })
    }

    fn candidate_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        if path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            anyhow::bail!("workspace path cannot contain '..': {}", path.display());
        }
        let root = self.workspace_root_canonical()?;
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        Ok(candidate)
    }

    fn resolve_existing_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let candidate = self.candidate_path(path)?;
        let resolved = candidate
            .canonicalize()
            .with_context(|| format!("path does not exist: {}", candidate.display()))?;
        if self.sandbox_config.sandbox_mode == SandboxMode::DangerFullAccess {
            return Ok(resolved);
        }
        let readable_roots = self.canonical_roots(
            self.sandbox_config
                .effective_readable_roots(&self.workspace_root),
        );
        if !readable_roots.iter().any(|root| resolved.starts_with(root)) {
            anyhow::bail!(
                "path is outside the workspace and no readable root authorized it: {}",
                path.display()
            );
        }
        Ok(resolved)
    }

    fn resolve_write_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        if self.sandbox_config.sandbox_mode == SandboxMode::ReadOnly {
            anyhow::bail!("sandbox mode read-only does not permit file writes");
        }
        let candidate = self.candidate_path(path)?;
        let mut ancestor = candidate.as_path();
        while !ancestor.exists() {
            ancestor = ancestor.parent().with_context(|| {
                format!(
                    "write path has no existing ancestor: {}",
                    candidate.display()
                )
            })?;
        }
        let suffix = candidate
            .strip_prefix(ancestor)
            .unwrap_or_else(|_| Path::new(""));
        let resolved_ancestor = ancestor.canonicalize()?;
        let resolved_candidate = resolved_ancestor.join(suffix);
        if self.sandbox_config.sandbox_mode == SandboxMode::DangerFullAccess {
            return Ok(candidate);
        }
        let writable_roots = self.canonical_roots(
            self.sandbox_config
                .effective_writable_roots(&self.workspace_root),
        );
        let Some(root) = writable_roots
            .iter()
            .find(|root| resolved_ancestor.starts_with(root.as_path()))
        else {
            anyhow::bail!("write path escapes workspace: {}", path.display());
        };
        if is_protected_metadata_path(&resolved_candidate, root)
            && !self
                .sandbox_config
                .is_approved_write_path(&resolved_candidate)
        {
            return Err(ApprovalRequired::new(format!(
                "Write to protected workspace metadata: {}",
                path.display()
            ))
            .into());
        }
        Ok(candidate)
    }

    fn canonical_roots(&self, roots: Vec<PathBuf>) -> Vec<PathBuf> {
        roots
            .into_iter()
            .filter_map(|root| root.canonicalize().ok())
            .collect()
    }

    fn register_process(&self, request_id: String, cancel: CancellationToken) {
        self.running.lock().unwrap().insert(request_id, cancel);
    }

    fn unregister_process(&self, request_id: &str) {
        self.running.lock().unwrap().remove(request_id);
    }
}

#[async_trait]
impl ExecutionEnvironment for LocalExecutionEnvironment {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> ExecutionEnvironmentKind {
        ExecutionEnvironmentKind::Local
    }

    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn resolve_read_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        self.resolve_existing_path(path)
    }

    async fn exec(
        &self,
        request: ExecRequest,
        context: ExecutionContext,
    ) -> anyhow::Result<ExecResult> {
        let cwd = request
            .cwd
            .as_deref()
            .map(|path| self.resolve_existing_path(path))
            .transpose()?
            .unwrap_or(self.workspace_root_canonical()?);

        let command_plan = build_local_sandbox_command(
            &request.program,
            &request.args,
            &cwd,
            &self.workspace_root,
            &self.sandbox_config,
        )?;
        let sandbox = Some(ExecutionSandboxMetadata {
            status: command_plan.status.clone(),
            permission_profile: sandbox_permission_profile(
                OsSandboxPlatform::current(),
                &self.sandbox_config,
            ),
            sandbox_mode: self.sandbox_config.sandbox_mode,
            network: self.sandbox_config.network,
        });

        if let SandboxCommandStatus::BestEffortPassthrough { platform, reason } =
            &command_plan.status
        {
            tracing::warn!(
                platform = platform.as_str(),
                reason = %reason,
                "local sandbox best_effort is running without OS-level isolation"
            );
        }

        let mut process = Command::new(&command_plan.program);
        if request.clear_env {
            process.env_clear();
        } else {
            for key in SENSITIVE_CHILD_ENV_KEYS {
                process.env_remove(key);
            }
        }
        process
            .args(&command_plan.args)
            .envs(&request.env)
            .envs(command_plan.env.iter().cloned())
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if request.stdin.is_some() {
            process.stdin(Stdio::piped());
        }

        let mut child = process
            .spawn()
            .with_context(|| format!("failed to spawn {}", command_plan.program))?;

        let request_id = uuid::Uuid::new_v4().to_string();
        let cancel_token = CancellationToken::new();
        self.register_process(request_id.clone(), cancel_token.clone());

        if let Some(stdin) = request.stdin {
            if let Some(mut child_stdin) = child.stdin.take() {
                child_stdin
                    .write_all(&stdin)
                    .await
                    .with_context(|| format!("failed to write stdin for {}", request.program))?;
                let _ = child_stdin.shutdown().await;
            }
        }

        let max_bytes = context.resource_limits.max_output_bytes;
        let output_limit_reached = CancellationToken::new();

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();

        let read_stdout = {
            let limit = output_limit_reached.clone();
            let max = max_bytes;
            async move {
                match stdout_pipe {
                    Some(pipe) => read_pipe_with_limit(pipe, max, limit).await,
                    None => (Vec::new(), false),
                }
            }
        };
        let read_stderr = {
            let limit = output_limit_reached.clone();
            let max = max_bytes;
            async move {
                match stderr_pipe {
                    Some(pipe) => read_pipe_with_limit(pipe, max, limit).await,
                    None => (Vec::new(), false),
                }
            }
        };

        let stdout_handle = tokio::spawn(read_stdout);
        let stderr_handle = tokio::spawn(read_stderr);

        enum WaitOutcome {
            Exited(std::process::ExitStatus),
            Cancelled(String),
            OutputLimitExceeded,
            TimedOut(String),
        }

        let wait_outcome: anyhow::Result<WaitOutcome> = {
            let ctx_cancel = context.cancel.clone();
            let reg_cancel = cancel_token.clone();
            let limit_reached = output_limit_reached.clone();
            let timeout_dur = context.timeout;
            let program = command_plan.program.clone();

            tokio::select! {
                result = child.wait() => {
                    result
                        .with_context(|| format!("{} process wait failed", program))
                        .map(WaitOutcome::Exited)
                }
                _ = async {
                    if let Some(token) = ctx_cancel {
                        token.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Ok(WaitOutcome::Cancelled("execution cancelled by context".to_string()))
                }
                _ = reg_cancel.cancelled() => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Ok(WaitOutcome::Cancelled("execution cancelled by request_id".to_string()))
                }
                _ = limit_reached.cancelled() => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Ok(WaitOutcome::OutputLimitExceeded)
                }
                _ = tokio::time::sleep(timeout_dur) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Ok(WaitOutcome::TimedOut(format!(
                        "{} timed out after {:?}",
                        program, timeout_dur
                    )))
                }
            }
        };

        let (stdout, stdout_truncated) = stdout_handle.await.unwrap_or_default();
        let (stderr, stderr_truncated) = stderr_handle.await.unwrap_or_default();

        let truncated = stdout_truncated || stderr_truncated || output_limit_reached.is_cancelled();

        self.unregister_process(&request_id);

        let wait_outcome = wait_outcome?;

        let mut result = match wait_outcome {
            WaitOutcome::Exited(exit_status) => ExecResult {
                stdout,
                stderr,
                exit_code: exit_status.code(),
                success: exit_status.success(),
                truncated,
                sandbox: sandbox.clone(),
            },
            WaitOutcome::OutputLimitExceeded => ExecResult {
                stdout,
                stderr,
                exit_code: None,
                success: false,
                truncated: true,
                sandbox,
            },
            WaitOutcome::Cancelled(reason) | WaitOutcome::TimedOut(reason) => {
                anyhow::bail!("{reason}");
            }
        };

        if result.truncated {
            if let Some(max) = max_bytes {
                result.stdout = truncate_output_vec(result.stdout, Some(max));
                result.stderr = truncate_output_vec(result.stderr, Some(max));
            }
        }

        Ok(result)
    }

    async fn spawn_stdio(
        &self,
        request: ExecRequest,
        context: ExecutionContext,
    ) -> anyhow::Result<Box<dyn StdioSession>> {
        let cwd = request
            .cwd
            .as_deref()
            .map(|path| self.resolve_existing_path(path))
            .transpose()?
            .unwrap_or(self.workspace_root_canonical()?);

        let command_plan = build_local_sandbox_command(
            &request.program,
            &request.args,
            &cwd,
            &self.workspace_root,
            &self.sandbox_config,
        )?;
        let sandbox = Some(ExecutionSandboxMetadata {
            status: command_plan.status.clone(),
            permission_profile: sandbox_permission_profile(
                OsSandboxPlatform::current(),
                &self.sandbox_config,
            ),
            sandbox_mode: self.sandbox_config.sandbox_mode,
            network: self.sandbox_config.network,
        });
        let mut process = Command::new(&command_plan.program);
        if request.clear_env {
            process.env_clear();
        } else {
            for key in SENSITIVE_CHILD_ENV_KEYS {
                process.env_remove(key);
            }
        }
        process
            .args(&command_plan.args)
            .envs(&request.env)
            .envs(command_plan.env.iter().cloned())
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = process
            .spawn()
            .with_context(|| format!("failed to spawn {}", command_plan.program))?;

        let child_stdin = child
            .stdin
            .take()
            .with_context(|| format!("failed to open stdin for {}", request.program))?;
        let child_stdout = child
            .stdout
            .take()
            .with_context(|| format!("failed to open stdout for {}", request.program))?;
        let child_stderr = child
            .stderr
            .take()
            .with_context(|| format!("failed to open stderr for {}", request.program))?;

        let request_id = uuid::Uuid::new_v4().to_string();
        let cancel_token = CancellationToken::new();
        self.register_process(request_id.clone(), cancel_token.clone());

        Ok(Box::new(LocalStdioSession {
            child: tokio::sync::Mutex::new(Some(child)),
            stdin: tokio::sync::Mutex::new(child_stdin),
            stdout: tokio::sync::Mutex::new(child_stdout),
            stderr: tokio::sync::Mutex::new(child_stderr),
            cancel: context.cancel,
            cancel_token: Some(cancel_token),
            request_id: Some(request_id),
            env: Some(Arc::new(self.clone())),
            sandbox,
        }))
    }

    async fn cancel(&self, request_id: &str) -> anyhow::Result<()> {
        let mut running = self.running.lock().unwrap();
        if let Some(token) = running.remove(request_id) {
            token.cancel();
            Ok(())
        } else {
            anyhow::bail!("no running process found for request_id: {}", request_id)
        }
    }

    async fn read_file(&self, request: FileReadRequest) -> anyhow::Result<FileReadResult> {
        let path = self.resolve_existing_path(&request.path)?;
        if let Some(max_bytes) = request.max_bytes {
            let metadata = tokio::fs::metadata(&path)
                .await
                .with_context(|| format!("failed to inspect {}", path.display()))?;
            if metadata.len() > max_bytes {
                anyhow::bail!(
                    "file {} is {} bytes; read limit is {} bytes",
                    path.display(),
                    metadata.len(),
                    max_bytes
                );
            }
        }
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("failed to read {}", path.display()))?;
        if request
            .max_bytes
            .is_some_and(|max_bytes| bytes.len() as u64 > max_bytes)
        {
            anyhow::bail!("file {} exceeded the configured read limit", path.display());
        }
        Ok(FileReadResult { path, bytes })
    }

    async fn write_file(&self, request: FileWriteRequest) -> anyhow::Result<WriteResult> {
        let path = self.resolve_write_path(&request.path)?;
        if request.create_parent_dirs {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let bytes_written = request.contents.len();
        tokio::fs::write(&path, request.contents)
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(WriteResult {
            path,
            bytes_written,
        })
    }
}

async fn read_pipe_with_limit<R: AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: Option<usize>,
    limit_reached: CancellationToken,
) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        tokio::select! {
            result = reader.read(&mut buf) => {
                match result {
                    Ok(0) => break,
                    Ok(n) => {
                        output.extend_from_slice(&buf[..n]);
                        if let Some(max) = max_bytes {
                            if output.len() > max {
                                limit_reached.cancel();
                                return (output, true);
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            _ = limit_reached.cancelled() => {
                return (output, true);
            }
        }
    }
    (output, false)
}

fn truncate_output_vec(bytes: Vec<u8>, max_bytes: Option<usize>) -> Vec<u8> {
    match max_bytes {
        Some(max) if bytes.len() > max => {
            let mut truncated = bytes[..max].to_vec();
            truncated.extend_from_slice(b"\n\n[output truncated by resource limit]");
            truncated
        }
        _ => bytes,
    }
}

pub struct LocalStdioSession {
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    stdout: tokio::sync::Mutex<tokio::process::ChildStdout>,
    stderr: tokio::sync::Mutex<tokio::process::ChildStderr>,
    cancel: Option<CancellationToken>,
    cancel_token: Option<CancellationToken>,
    request_id: Option<String>,
    env: Option<std::sync::Arc<LocalExecutionEnvironment>>,
    sandbox: Option<ExecutionSandboxMetadata>,
}

#[async_trait]
impl StdioSession for LocalStdioSession {
    async fn write_stdin(&self, data: &[u8]) -> anyhow::Result<()> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(data).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_stdout(&self) -> anyhow::Result<Vec<u8>> {
        let mut stdout = self.stdout.lock().await;
        let mut buf = vec![0u8; 8192];
        let bytes_read = stdout.read(&mut buf).await?;
        buf.truncate(bytes_read);
        Ok(buf)
    }

    async fn read_stderr(&self) -> anyhow::Result<Vec<u8>> {
        let mut stderr = self.stderr.lock().await;
        let mut buf = vec![0u8; 8192];
        let bytes_read = stderr.read(&mut buf).await?;
        buf.truncate(bytes_read);
        Ok(buf)
    }

    async fn close(&self) -> anyhow::Result<ExecResult> {
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }

        let mut child_guard = self.child.lock().await;
        let mut child = child_guard.take();

        if let Some(ref mut child) = child {
            let wait_result = match (&self.cancel, &self.cancel_token) {
                (Some(cancel), Some(cancel_token)) => {
                    let cancel = cancel.clone();
                    let cancel_token = cancel_token.clone();
                    tokio::select! {
                        result = child.wait() => result,
                        _ = cancel.cancelled() => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            anyhow::bail!("stdio session cancelled during close");
                        }
                        _ = cancel_token.cancelled() => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            anyhow::bail!("stdio session cancelled during close");
                        }
                    }
                }
                (Some(cancel), None) => {
                    let cancel = cancel.clone();
                    tokio::select! {
                        result = child.wait() => result,
                        _ = cancel.cancelled() => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            anyhow::bail!("stdio session cancelled during close");
                        }
                    }
                }
                (None, Some(cancel_token)) => {
                    let cancel_token = cancel_token.clone();
                    tokio::select! {
                        result = child.wait() => result,
                        _ = cancel_token.cancelled() => {
                            let _ = child.kill().await;
                            let _ = child.wait().await;
                            anyhow::bail!("stdio session cancelled during close");
                        }
                    }
                }
                (None, None) => child.wait().await,
            };

            if let Some(ref request_id) = self.request_id {
                if let Some(ref env) = self.env {
                    env.unregister_process(request_id);
                }
            }

            let exit_status = wait_result?;
            return Ok(ExecResult {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: exit_status.code(),
                success: exit_status.success(),
                truncated: false,
                sandbox: self.sandbox.clone(),
            });
        }

        if let Some(ref request_id) = self.request_id {
            if let Some(ref env) = self.env {
                env.unregister_process(request_id);
            }
        }

        Ok(ExecResult {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: None,
            success: true,
            truncated: false,
            sandbox: self.sandbox.clone(),
        })
    }

    async fn kill(&self) -> anyhow::Result<()> {
        if let Some(cancel_token) = &self.cancel_token {
            cancel_token.cancel();
        }

        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        if let Some(ref request_id) = self.request_id {
            if let Some(ref env) = self.env {
                env.unregister_process(request_id);
            }
        }

        Ok(())
    }

    fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn local_environment_reads_writes_and_execs() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let written = env
            .write_file(FileWriteRequest::new("nested/hello.txt", b"hello".to_vec()))
            .await
            .expect("write file");
        assert_eq!(written.bytes_written, 5);

        let read = env
            .read_file(FileReadRequest::new("nested/hello.txt"))
            .await
            .expect("read file");
        assert_eq!(read.bytes, b"hello");

        let limited = env
            .read_file(FileReadRequest::new("nested/hello.txt").with_max_bytes(4))
            .await
            .expect_err("bounded read should reject an oversized file");
        assert!(limited.to_string().contains("read limit"));

        let command = if cfg!(windows) {
            "Write-Output ok"
        } else {
            "printf ok"
        };
        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(5)),
            )
            .await
            .expect("exec shell command");
        assert!(exec.success);
        assert!(String::from_utf8_lossy(&exec.stdout).contains("ok"));
        assert!(matches!(
            exec.sandbox
                .expect("execution records sandbox metadata")
                .status,
            SandboxCommandStatus::Disabled
        ));

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn relative_paths_and_default_shell_cwd_are_workspace_scoped() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-workspace-cwd-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("nested")).expect("create temp workspace");
        std::fs::write(root.join("nested/value.txt"), "workspace").expect("write fixture");
        let env = LocalExecutionEnvironment::new(root.clone());

        let read = env
            .read_file(FileReadRequest::new("nested/value.txt"))
            .await
            .expect("relative read resolves from workspace root");
        assert_eq!(
            read.path,
            root.join("nested/value.txt").canonicalize().unwrap()
        );

        let command = if cfg!(windows) {
            "(Get-Location).Path"
        } else {
            "pwd -P"
        };
        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect("shell starts in workspace root");
        assert!(exec.success);
        let reported_cwd = PathBuf::from(String::from_utf8_lossy(&exec.stdout).trim())
            .canonicalize()
            .expect("reported shell cwd exists");
        assert_eq!(
            reported_cwd,
            root.canonicalize().expect("canonical workspace root")
        );

        let nested_exec = env
            .exec(
                ExecRequest::shell(command).cwd("nested"),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect("relative shell cwd resolves from workspace root");
        assert!(nested_exec.success);
        let reported_nested_cwd =
            PathBuf::from(String::from_utf8_lossy(&nested_exec.stdout).trim())
                .canonicalize()
                .expect("reported nested shell cwd exists");
        assert_eq!(
            reported_nested_cwd,
            root.join("nested")
                .canonicalize()
                .expect("canonical nested cwd")
        );

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn parent_paths_are_blocked_but_configured_readable_roots_remain_available() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-scope-root-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-core-scope-outside-{id}"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        std::fs::create_dir_all(&outside).expect("create additional readable root");
        std::fs::write(outside.join("allowed.txt"), "allowed").expect("write outside fixture");

        let env = LocalExecutionEnvironment::new(root.clone());
        let traversal = env
            .read_file(FileReadRequest::new("../.."))
            .await
            .expect_err("parent traversal must be rejected");
        assert!(traversal.to_string().contains("cannot contain '..'"));

        let absolute_parent = env
            .read_file(FileReadRequest::new(outside.join("allowed.txt")))
            .await
            .expect_err("unconfigured absolute parent path must be rejected");
        assert!(absolute_parent
            .to_string()
            .contains("no readable root authorized"));

        let parent_cwd = env
            .exec(
                ExecRequest::shell(if cfg!(windows) {
                    "Write-Output blocked"
                } else {
                    "printf blocked"
                })
                .cwd(&outside),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect_err("unconfigured shell cwd must be rejected");
        assert!(parent_cwd
            .to_string()
            .contains("no readable root authorized"));

        let mut config = LocalSandboxConfig::default();
        config.read_paths = vec![outside.clone()];
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);
        let read = env
            .read_file(FileReadRequest::new(outside.join("allowed.txt")))
            .await
            .expect("configured readable root remains available");
        assert_eq!(read.bytes, b"allowed");
        let exec = env
            .exec(
                ExecRequest::shell(if cfg!(windows) {
                    "Write-Output allowed"
                } else {
                    "printf allowed"
                })
                .cwd(&outside),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect("configured readable root is a valid shell cwd");
        assert!(exec.success);

        std::fs::remove_dir_all(root).expect("remove temp workspace");
        std::fs::remove_dir_all(outside).expect("remove additional readable root");
    }

    #[tokio::test]
    async fn read_only_environment_rejects_builtin_file_writes() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-read-only-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let config = LocalSandboxConfig::enforce().with_sandbox_mode(SandboxMode::ReadOnly);
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);

        let error = env
            .write_file(FileWriteRequest::new("blocked.txt", b"blocked".to_vec()))
            .await
            .expect_err("read-only mode must reject writes");

        assert!(error.to_string().contains("read-only"));
        assert!(!root.join("blocked.txt").exists());
        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn workspace_write_allows_configured_writable_root() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-workspace-{id}"));
        let extra = std::env::temp_dir().join(format!("opentopia-core-extra-{id}"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        std::fs::create_dir_all(&extra).expect("create extra writable root");
        let mut config = LocalSandboxConfig::default();
        config.writable_roots = vec![extra.clone()];
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);

        env.write_file(FileWriteRequest::new(
            extra.join("allowed.txt"),
            b"allowed".to_vec(),
        ))
        .await
        .expect("write additional root");

        assert!(extra.join("allowed.txt").exists());
        std::fs::remove_dir_all(root).expect("remove temp workspace");
        std::fs::remove_dir_all(extra).expect("remove extra writable root");
    }

    #[tokio::test]
    async fn workspace_write_protects_agent_metadata() {
        let root = std::env::temp_dir().join(format!(
            "opentopia-core-protected-metadata-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let error = env
            .write_file(FileWriteRequest::new(
                ".codex/config.toml",
                b"unsafe".to_vec(),
            ))
            .await
            .expect_err("protected metadata must remain read-only");

        assert!(error.to_string().contains("protected workspace metadata"));
        assert!(!root.join(".codex/config.toml").exists());
        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn danger_full_access_allows_builtin_write_outside_workspace() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-full-root-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-core-full-outside-{id}.txt"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let config = LocalSandboxConfig::default().with_sandbox_mode(SandboxMode::DangerFullAccess);
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);

        env.write_file(FileWriteRequest::new(&outside, b"allowed".to_vec()))
            .await
            .expect("full access write outside workspace");

        assert!(outside.exists());
        std::fs::remove_dir_all(root).expect("remove temp workspace");
        std::fs::remove_file(outside).expect("remove outside file");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_environment_windows_best_effort_sandbox_executes() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::with_sandbox_config(
            root.clone(),
            LocalSandboxConfig::best_effort(),
        );

        let exec = env
            .exec(
                ExecRequest::shell("Write-Output ok"),
                ExecutionContext::with_timeout(Duration::from_secs(45)),
            )
            .await
            .expect("windows restricted-token sandbox should run");

        assert!(exec.success);
        assert!(String::from_utf8_lossy(&exec.stdout).contains("ok"));
        let sandbox = exec.sandbox.expect("execution records sandbox metadata");
        assert_eq!(sandbox.permission_profile, ":workspace");
        assert!(matches!(
            sandbox.status,
            SandboxCommandStatus::Wrapped {
                platform: OsSandboxPlatform::Windows,
                ..
            }
        ));

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_environment_windows_enforced_sandbox_denies_outside_write() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-sandbox-{id}"));
        let outside = std::env::current_dir()
            .expect("current directory")
            .parent()
            .expect("workspace parent")
            .join(format!("opentopia-core-outside-{id}.txt"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::with_sandbox_config(
            root.clone(),
            LocalSandboxConfig::enforce(),
        );
        let escaped_outside = outside.to_string_lossy().replace("'", "''");
        let command = format!(
            "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{escaped_outside}' -Value blocked"
        );

        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect("restricted-token sandbox command should start");

        let outside_was_written = outside.exists();
        let command_succeeded = exec.success;
        std::fs::remove_dir_all(root).expect("remove temp workspace");
        let _ = std::fs::remove_file(outside);
        assert!(!outside_was_written, "sandbox wrote outside the workspace");
        assert!(
            !command_succeeded,
            "outside write should fail in enforced mode"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_environment_windows_read_only_denies_workspace_write() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-readonly-{id}"));
        let sandbox_home = std::env::temp_dir().join(format!("opentopia-core-readonly-home-{id}"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let mut config = LocalSandboxConfig::enforce().with_sandbox_mode(SandboxMode::ReadOnly);
        config.sandbox_home = Some(sandbox_home.clone());
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);
        let target = root.join("blocked.txt");
        let command = format!(
            "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value blocked",
            target.to_string_lossy().replace('\'', "''")
        );

        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect("read-only sandbox command should start");

        assert!(!exec.success, "read-only command unexpectedly wrote a file");
        assert!(!target.exists());
        std::fs::remove_dir_all(root).expect("remove temp workspace");
        let _ = std::fs::remove_dir_all(sandbox_home);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_environment_windows_allows_additional_writable_root() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-root-{id}"));
        let extra = std::env::temp_dir().join(format!("opentopia-core-writable-{id}"));
        let sandbox_home = std::env::temp_dir().join(format!("opentopia-core-writable-home-{id}"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        std::fs::create_dir_all(&extra).expect("create extra writable root");
        let mut config = LocalSandboxConfig::enforce();
        config.writable_roots = vec![extra.clone()];
        config.sandbox_home = Some(sandbox_home.clone());
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);
        let target = extra.join("allowed.txt");
        let command = format!(
            "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value allowed",
            target.to_string_lossy().replace('\'', "''")
        );

        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(30)),
            )
            .await
            .expect("workspace-write sandbox command should start");

        assert!(
            exec.success,
            "additional writable root failed: {}",
            String::from_utf8_lossy(&exec.stderr)
        );
        assert!(target.exists());
        std::fs::remove_dir_all(root).expect("remove temp workspace");
        std::fs::remove_dir_all(extra).expect("remove extra writable root");
        let _ = std::fs::remove_dir_all(sandbox_home);
    }

    #[tokio::test]
    async fn local_environment_respects_max_output_bytes() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let command = if cfg!(windows) {
            "\"hello world!\""
        } else {
            "echo hello world!"
        };
        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(5)).with_resource_limits(
                    ResourceLimit {
                        max_output_bytes: Some(4),
                        ..Default::default()
                    },
                ),
            )
            .await
            .expect("exec shell command");
        let stdout = String::from_utf8_lossy(&exec.stdout);
        assert!(
            stdout.contains("truncated"),
            "expected truncation marker in: {stdout:?}"
        );

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn local_environment_cancellation() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            let command = if cfg!(windows) {
                "Start-Sleep -Seconds 30"
            } else {
                "sleep 30"
            };
            env.exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(60)).with_cancel(cancel_clone),
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel.cancel();

        let result = handle.await.expect("join");
        assert!(result.is_err(), "expected cancellation error");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cancelled"), "error: {err}");

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn local_environment_cancel_by_request_id() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = std::sync::Arc::new(LocalExecutionEnvironment::new(root.clone()));

        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };

        let env_clone = env.clone();
        let handle = tokio::spawn(async move {
            env_clone
                .exec(
                    ExecRequest::shell(command),
                    ExecutionContext::with_timeout(Duration::from_secs(60)),
                )
                .await
        });

        tokio::time::sleep(Duration::from_millis(500)).await;
        let request_ids: Vec<String> = { env.running.lock().unwrap().keys().cloned().collect() };
        assert!(
            !request_ids.is_empty(),
            "expected at least one running process"
        );

        for rid in &request_ids {
            env.cancel(rid).await.expect("cancel should succeed");
        }

        let result = handle.await.expect("join");
        assert!(result.is_err(), "expected cancellation error");

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn local_environment_truncated_flag() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let command = if cfg!(windows) {
            "\"hello world!\""
        } else {
            "echo hello world!"
        };
        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(5)).with_resource_limits(
                    ResourceLimit {
                        max_output_bytes: Some(4),
                        ..Default::default()
                    },
                ),
            )
            .await
            .expect("exec shell command");
        assert!(exec.truncated, "expected truncated flag");

        let exec2 = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(5)),
            )
            .await
            .expect("exec shell command");
        assert!(!exec2.truncated, "expected no truncated flag");

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    #[tokio::test]
    async fn local_environment_spawn_stdio() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let program = if cfg!(windows) {
            "powershell.exe"
        } else {
            "sh"
        };
        let arg = if cfg!(windows) { "-Command" } else { "-c" };
        let script = if cfg!(windows) {
            "$line = [Console]::In.ReadLine(); \"you said: $line\""
        } else {
            "read line; echo \"you said: $line\""
        };

        let session = env
            .spawn_stdio(
                ExecRequest::new(program).arg(arg).arg(script),
                ExecutionContext::with_timeout(Duration::from_secs(10)),
            )
            .await
            .expect("spawn stdio");

        session.write_stdin(b"hello\n").await.expect("write stdin");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let reply_bytes = session.read_stdout().await.expect("read stdout");
        let reply = String::from_utf8_lossy(&reply_bytes);
        assert!(reply.contains("hello"), "reply: {reply}");

        let result = session.close().await.expect("close session");
        assert!(result.success || result.exit_code == Some(0));

        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }
}
