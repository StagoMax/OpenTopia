use crate::execution_authorization::ProcessLifetime;
use crate::execution_runtime::{
    configure_command_environment, configure_stdio, environment_keys, resolve_runtime,
};
pub use crate::execution_spec::{
    shell_command_compatibility_error, EnvironmentPolicy, ExecRequest, ExecutionFailure,
    ExecutionRequirements, ExecutionSpec, ExecutionStage, LifecyclePolicy, RuntimeRequirements,
    ShellCompatibilityError, ShellDialect, StdioPolicy,
};
use crate::policy::ApprovalRequired;
use crate::process_quota::ProcessQuota;
use crate::process_supervisor::{spawn_process, terminate_process};
use crate::sandbox::{
    build_local_sandbox_command_with_options, is_protected_metadata_path,
    sandbox_permission_profile, ExecutionEnvironmentKind, LocalSandboxConfig, NetworkPolicy,
    OsSandboxPlatform, SandboxBackendCapabilities, SandboxCommandStatus, SandboxLaunchOptions,
    SandboxMode, SandboxPreparationPlan,
};
use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const SANDBOX_ERROR_EXIT_CODE: i32 = 125;
const SANDBOX_ERROR_PREFIX: &str = "OPENTOPIA_SANDBOX_ERROR ";
const SANDBOX_ERROR_NONCE_ENV: &str = "OPENTOPIA_SANDBOX_ERROR_NONCE";

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

/// Which stream a chunk of live command output came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// Receives command output while the command is still running.
///
/// `exec` normally returns output only once the process exits, which is fine for a
/// command that finishes in seconds and useless for one that runs for an hour. A
/// sink makes the same execution path observable as it happens, so a long command
/// can report progress without being treated differently from a short one.
pub trait BackgroundOutputSink: std::fmt::Debug + Send + Sync {
    fn push(&self, stream: OutputStream, chunk: &[u8]);
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub timeout: Duration,
    pub startup_timeout: Duration,
    pub termination_timeout: Duration,
    pub cancel: Option<CancellationToken>,
    pub resource_limits: ResourceLimit,
    /// Lifecycle is orthogonal to RPC/command timeout. Persistent services are
    /// stopped only by their owner, never because one request deadline elapsed.
    pub process_lifetime: ProcessLifetime,
    /// When set, output is forwarded here as it arrives as well as being returned.
    pub output_sink: Option<Arc<dyn BackgroundOutputSink>>,
}

impl ExecutionContext {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            startup_timeout: Duration::from_secs(15),
            termination_timeout: Duration::from_secs(5),
            cancel: None,
            resource_limits: ResourceLimit::default(),
            process_lifetime: ProcessLifetime::OneShot,
            output_sink: None,
        }
    }

    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn with_startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    pub fn with_termination_timeout(mut self, timeout: Duration) -> Self {
        self.termination_timeout = timeout;
        self
    }

    pub fn with_resource_limits(mut self, limits: ResourceLimit) -> Self {
        self.resource_limits = limits;
        self
    }

    pub fn with_process_lifetime(mut self, lifetime: ProcessLifetime) -> Self {
        self.process_lifetime = lifetime;
        self
    }

    pub fn with_output_sink(mut self, sink: Arc<dyn BackgroundOutputSink>) -> Self {
        self.output_sink = Some(sink);
        self
    }
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            startup_timeout: Duration::from_secs(15),
            termination_timeout: Duration::from_secs(5),
            cancel: None,
            resource_limits: ResourceLimit::default(),
            process_lifetime: ProcessLifetime::OneShot,
            output_sink: None,
        }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone)]
pub struct FileDeleteRequest {
    pub path: PathBuf,
}

impl FileDeleteRequest {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[derive(Debug, Clone)]
pub struct DeleteResult {
    pub path: PathBuf,
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
    /// Wait for the process without closing stdin first.
    ///
    /// Persistent shell sessions use this while another task continues to write
    /// input. Implementations that do not distinguish wait from close retain the
    /// old behavior through this conservative default.
    async fn wait(&self) -> anyhow::Result<ExecResult> {
        self.close().await
    }
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

    /// Whether `spawn_stdio` can remain alive while callers independently read,
    /// write, and wait for it. The default prevents a nominal stdio adapter from
    /// being exposed as an interactive session when its `wait` closes stdin.
    fn supports_persistent_stdio(&self) -> bool {
        false
    }

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
    async fn delete_file(&self, _request: FileDeleteRequest) -> anyhow::Result<DeleteResult> {
        anyhow::bail!("this execution environment does not support direct file deletion")
    }

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
    prepared_sandbox_scopes: Arc<Mutex<HashSet<String>>>,
    sandbox_preparation_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl LocalExecutionEnvironment {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            id: "local".to_string(),
            workspace_root: workspace_root.into(),
            sandbox_config: LocalSandboxConfig::default(),
            running: Arc::new(Mutex::new(HashMap::new())),
            prepared_sandbox_scopes: Arc::new(Mutex::new(HashSet::new())),
            sandbox_preparation_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_id(id: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            id: id.into(),
            workspace_root: workspace_root.into(),
            sandbox_config: LocalSandboxConfig::default(),
            running: Arc::new(Mutex::new(HashMap::new())),
            prepared_sandbox_scopes: Arc::new(Mutex::new(HashSet::new())),
            sandbox_preparation_locks: Arc::new(Mutex::new(HashMap::new())),
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
            prepared_sandbox_scopes: Arc::new(Mutex::new(HashSet::new())),
            sandbox_preparation_locks: Arc::new(Mutex::new(HashMap::new())),
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
            prepared_sandbox_scopes: Arc::new(Mutex::new(HashSet::new())),
            sandbox_preparation_locks: Arc::new(Mutex::new(HashMap::new())),
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
                .configured_readable_roots(&self.workspace_root),
        );
        let approved = self.sandbox_config.is_within_approved_read_scope(&resolved)
            || self
                .sandbox_config
                .is_within_approved_write_scope(&resolved);
        if !approved && !readable_roots.iter().any(|root| resolved.starts_with(root)) {
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
        let approved = self
            .sandbox_config
            .is_within_approved_write_scope(&resolved_candidate);
        let writable_roots = self.canonical_roots(
            self.sandbox_config
                .configured_writable_roots(&self.workspace_root),
        );
        let configured_root = writable_roots
            .iter()
            .find(|root| resolved_ancestor.starts_with(root.as_path()));
        if configured_root.is_none() && !approved {
            anyhow::bail!("write path escapes workspace: {}", path.display());
        }
        let root = configured_root.unwrap_or(&resolved_ancestor);
        if is_protected_metadata_path(&resolved_candidate, root) && !approved {
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

    async fn prepare_sandbox_scope(
        &self,
        preparation: Option<&SandboxPreparationPlan>,
        cwd: &Path,
        startup_timeout: Duration,
    ) -> anyhow::Result<()> {
        let Some(preparation) = preparation else {
            return Ok(());
        };
        if self
            .prepared_sandbox_scopes
            .lock()
            .unwrap()
            .contains(&preparation.key)
        {
            return Ok(());
        }
        let scope_lock = self
            .sandbox_preparation_locks
            .lock()
            .unwrap()
            .entry(preparation.key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _scope_guard = scope_lock.lock().await;
        if self
            .prepared_sandbox_scopes
            .lock()
            .unwrap()
            .contains(&preparation.key)
        {
            return Ok(());
        }

        let mut command = Command::new(&preparation.program);
        command
            .args(&preparation.args)
            .envs(preparation.env.iter().cloned())
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let timeout = startup_timeout.max(Duration::from_secs(120));
        let output = tokio::time::timeout(timeout, command.output())
            .await
            .map_err(|_| {
                ExecutionFailure::without_os_error(
                    ExecutionStage::PrepareSandbox,
                    format!(
                        "sandbox ACL preparation timed out after {timeout:?}; no target command was started"
                    ),
                )
            })?
            .with_context(|| {
                format!(
                    "failed to start sandbox preparation helper {}",
                    preparation.program
                )
            })?;
        if !output.status.success() {
            let expected_nonce = preparation
                .env
                .iter()
                .find(|(key, _)| key == SANDBOX_ERROR_NONCE_ENV)
                .map(|(_, value)| value.as_str());
            let message = sandbox_infrastructure_error(&output.stderr, expected_nonce)
                .unwrap_or_else(|| String::from_utf8_lossy(&output.stderr).trim().to_string());
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::PrepareSandbox,
                if message.is_empty() {
                    format!("sandbox ACL preparation exited with {}", output.status)
                } else {
                    message
                },
            )
            .into());
        }
        self.prepared_sandbox_scopes
            .lock()
            .unwrap()
            .insert(preparation.key.clone());
        Ok(())
    }

    fn validate_execution_requirements(&self, request: &ExecRequest) -> anyhow::Result<()> {
        let capabilities = SandboxBackendCapabilities::for_platform(
            OsSandboxPlatform::current(),
            self.sandbox_config.effective_windows_backend(),
        );
        if !request.requirements.deny_read_paths.is_empty() && !capabilities.deny_read {
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::ValidatePolicy,
                "deny-read requirements need a sandbox backend with authoritative read isolation",
            )
            .into());
        }
        if !request.requirements.deny_write_paths.is_empty() && !capabilities.deny_write {
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::ValidatePolicy,
                "deny-write requirements are unsupported by the selected sandbox backend",
            )
            .into());
        }
        if request.requirements.network == Some(NetworkPolicy::Allow)
            && self.sandbox_config.network == NetworkPolicy::Deny
        {
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::ValidatePolicy,
                "the command requires network access but the sandbox is offline",
            )
            .into());
        }
        if request.requirements.network == Some(NetworkPolicy::Deny)
            && !capabilities.network_offline
        {
            return Err(ExecutionFailure::without_os_error(
                ExecutionStage::ValidatePolicy,
                "offline execution requires a sandbox backend with authoritative network isolation",
            )
            .into());
        }
        for path in &request.requirements.read_paths {
            self.resolve_existing_path(path)?;
        }
        for path in &request.requirements.deny_read_paths {
            self.resolve_existing_path(path)?;
        }
        for path in &request.requirements.write_paths {
            self.resolve_write_path(path)?;
        }
        Ok(())
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

    fn supports_persistent_stdio(&self) -> bool {
        true
    }

    fn resolve_read_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        self.resolve_existing_path(path)
    }

    async fn exec(
        &self,
        request: ExecRequest,
        context: ExecutionContext,
    ) -> anyhow::Result<ExecResult> {
        let initial_cwd = request
            .cwd
            .clone()
            .unwrap_or_else(|| self.workspace_root.clone());
        let request = crate::tool_adapter::adapt(request, &initial_cwd);
        self.validate_execution_requirements(&request)?;
        let cwd = request
            .cwd
            .as_deref()
            .map(|path| self.resolve_existing_path(path))
            .transpose()?
            .unwrap_or(self.workspace_root_canonical()?);

        let runtime = resolve_runtime(&request, &cwd, &self.workspace_root, &self.sandbox_config)?;
        let program = runtime.program.to_string_lossy().into_owned();
        let mut effective_config = self.sandbox_config.clone();
        if request.requirements.network == Some(NetworkPolicy::Deny) {
            effective_config.network = NetworkPolicy::Deny;
        }

        let command_plan = build_local_sandbox_command_with_options(
            &program,
            &request.args,
            &cwd,
            &self.workspace_root,
            &effective_config,
            &SandboxLaunchOptions {
                interactive: false,
                runtime_read_roots: runtime.read_roots.clone(),
                environment_keys: environment_keys(&runtime),
                additional_denied_read_paths: request.requirements.deny_read_paths.clone(),
                additional_protected_paths: request.requirements.deny_write_paths.clone(),
                // A stdio session is a long-lived transport (for example an MCP
                // server), so the caller's timeout applies to startup/handshake
                // work, not to the lifetime of the spawned process. The session
                // owner closes or kills it explicitly.
                timeout_ms: None,
                termination_timeout_ms: Some(
                    context
                        .termination_timeout
                        .as_millis()
                        .min(u64::MAX as u128) as u64,
                ),
                max_memory_bytes: context.resource_limits.max_memory_bytes,
                max_cpu_time_ms: context
                    .resource_limits
                    .max_cpu_time
                    .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64),
                max_output_bytes: context
                    .resource_limits
                    .max_output_bytes
                    .map(|bytes| bytes.min(u64::MAX as usize) as u64),
            },
        )?;
        let sandbox = Some(ExecutionSandboxMetadata {
            status: command_plan.status.clone(),
            permission_profile: sandbox_permission_profile(
                OsSandboxPlatform::current(),
                &effective_config,
            ),
            sandbox_mode: effective_config.sandbox_mode,
            network: effective_config.network,
        });
        let outer_wait_timeout = sandbox_outer_wait_timeout(
            &command_plan.status,
            context.timeout,
            context.termination_timeout,
        );

        if let SandboxCommandStatus::BestEffortPassthrough { platform, reason } =
            &command_plan.status
        {
            tracing::warn!(
                platform = platform.as_str(),
                reason = %reason,
                "local sandbox best_effort is running without OS-level isolation"
            );
        }

        self.prepare_sandbox_scope(
            command_plan.preparation.as_ref(),
            &cwd,
            context.startup_timeout,
        )
        .await?;

        let mut process = Command::new(&command_plan.program);
        configure_command_environment(&mut process, &request, &runtime, &effective_config);
        process
            .args(&command_plan.args)
            .envs(command_plan.env.iter().cloned())
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_stdio(&mut process, &request);

        // Held until the command finishes: the job terminates its members when the last
        // handle closes, so dropping this early would kill the command.
        let quota = if matches!(command_plan.status, SandboxCommandStatus::Wrapped { .. }) {
            None
        } else {
            ProcessQuota::prepare(&context.resource_limits)?
        };
        #[cfg(windows)]
        {
            let flags = crate::process_quota::suspended_creation_flags(quota.as_ref());
            if flags != 0 {
                process.creation_flags(flags);
            }
        }

        let mut child =
            spawn_process(process, context.startup_timeout, &command_plan.program).await?;

        if let Some(quota) = quota.as_ref() {
            // Fail closed. The process is still suspended, so killing it here means no
            // instruction of the unmetered command ever runs.
            if let Err(error) = quota.bind_and_resume(&child) {
                let _ = child.kill().await;
                return Err(error).with_context(|| {
                    format!(
                        "failed to apply the resource quota to {}",
                        command_plan.program
                    )
                });
            }
        }

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
            let sink = context.output_sink.clone();
            async move {
                match stdout_pipe {
                    Some(pipe) => {
                        read_pipe_with_limit(pipe, max, limit, sink, OutputStream::Stdout).await
                    }
                    None => (Vec::new(), false),
                }
            }
        };
        let read_stderr = {
            let limit = output_limit_reached.clone();
            let max = max_bytes;
            let sink = context.output_sink.clone();
            async move {
                match stderr_pipe {
                    Some(pipe) => {
                        read_pipe_with_limit(pipe, max, limit, sink, OutputStream::Stderr).await
                    }
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
            let timeout_dur = outer_wait_timeout;
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
                    terminate_process(&mut child, context.termination_timeout).await?;
                    Ok(WaitOutcome::Cancelled("execution cancelled by context".to_string()))
                }
                _ = reg_cancel.cancelled() => {
                    terminate_process(&mut child, context.termination_timeout).await?;
                    Ok(WaitOutcome::Cancelled("execution cancelled by request_id".to_string()))
                }
                _ = limit_reached.cancelled() => {
                    terminate_process(&mut child, context.termination_timeout).await?;
                    Ok(WaitOutcome::OutputLimitExceeded)
                }
                _ = tokio::time::sleep(timeout_dur) => {
                    terminate_process(&mut child, context.termination_timeout).await?;
                    Ok(WaitOutcome::TimedOut(format!(
                        "execution failed during wait: {} timed out after {:?}; process tree terminated",
                        program, timeout_dur
                    )))
                }
            }
        };

        // Closing the outer Job Object reclaims descendants and closes any
        // inherited output handles before the readers wait for EOF. This is
        // required even after the root process exits successfully.
        drop(quota);

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

        if matches!(command_plan.status, SandboxCommandStatus::Wrapped { .. })
            && result.exit_code == Some(SANDBOX_ERROR_EXIT_CODE)
        {
            let expected_nonce = command_plan
                .env
                .iter()
                .find(|(key, _)| key == SANDBOX_ERROR_NONCE_ENV)
                .map(|(_, value)| value.as_str());
            if let Some(message) = sandbox_infrastructure_error(&result.stderr, expected_nonce) {
                return Err(ExecutionFailure::without_os_error(
                    ExecutionStage::PrepareSandbox,
                    message,
                )
                .into());
            }
        }

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
        let initial_cwd = request
            .cwd
            .clone()
            .unwrap_or_else(|| self.workspace_root.clone());
        let request = crate::tool_adapter::adapt(request, &initial_cwd);
        self.validate_execution_requirements(&request)?;
        let cwd = request
            .cwd
            .as_deref()
            .map(|path| self.resolve_existing_path(path))
            .transpose()?
            .unwrap_or(self.workspace_root_canonical()?);

        let runtime = resolve_runtime(&request, &cwd, &self.workspace_root, &self.sandbox_config)?;
        let program = runtime.program.to_string_lossy().into_owned();

        let command_plan = build_local_sandbox_command_with_options(
            &program,
            &request.args,
            &cwd,
            &self.workspace_root,
            &self.sandbox_config,
            &SandboxLaunchOptions {
                interactive: false,
                runtime_read_roots: runtime.read_roots.clone(),
                environment_keys: environment_keys(&runtime),
                additional_denied_read_paths: request.requirements.deny_read_paths.clone(),
                additional_protected_paths: request.requirements.deny_write_paths.clone(),
                // A stdio session is a long-lived transport (for example an MCP
                // server). Its configured timeout bounds startup/handshake work,
                // not the lifetime of the process; the owner closes it explicitly.
                timeout_ms: None,
                termination_timeout_ms: Some(
                    context
                        .termination_timeout
                        .as_millis()
                        .min(u64::MAX as u128) as u64,
                ),
                max_memory_bytes: context.resource_limits.max_memory_bytes,
                max_cpu_time_ms: context
                    .resource_limits
                    .max_cpu_time
                    .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64),
                max_output_bytes: context
                    .resource_limits
                    .max_output_bytes
                    .map(|bytes| bytes.min(u64::MAX as usize) as u64),
            },
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
        let outer_wait_timeout = sandbox_outer_wait_timeout(
            &command_plan.status,
            context.timeout,
            context.termination_timeout,
        );
        self.prepare_sandbox_scope(
            command_plan.preparation.as_ref(),
            &cwd,
            context.startup_timeout,
        )
        .await?;
        let mut process = Command::new(&command_plan.program);
        configure_command_environment(&mut process, &request, &runtime, &self.sandbox_config);
        process
            .args(&command_plan.args)
            .envs(command_plan.env.iter().cloned())
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let quota = if matches!(command_plan.status, SandboxCommandStatus::Wrapped { .. }) {
            None
        } else {
            ProcessQuota::prepare(&context.resource_limits)?
        };
        #[cfg(windows)]
        {
            let flags = crate::process_quota::suspended_creation_flags(quota.as_ref());
            if flags != 0 {
                process.creation_flags(flags);
            }
        }

        let mut child =
            spawn_process(process, context.startup_timeout, &command_plan.program).await?;

        if let Some(quota) = quota.as_ref() {
            if let Err(error) = quota.bind_and_resume(&child) {
                let _ = child.kill().await;
                return Err(error).with_context(|| {
                    format!(
                        "failed to apply the resource quota to {}",
                        command_plan.program
                    )
                });
            }
        }

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
            sandbox_error_nonce: command_plan
                .env
                .iter()
                .find(|(key, _)| key == SANDBOX_ERROR_NONCE_ENV)
                .map(|(_, value)| value.clone()),
            stderr_observed: tokio::sync::Mutex::new(Vec::new()),
            execution_timeout: outer_wait_timeout,
            termination_timeout: context.termination_timeout,
            quota: tokio::sync::Mutex::new(quota),
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

    async fn delete_file(&self, request: FileDeleteRequest) -> anyhow::Result<DeleteResult> {
        let path = self.resolve_write_path(&request.path)?;
        let metadata = tokio::fs::metadata(&path)
            .await
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if !metadata.is_file() {
            anyhow::bail!("delete path is not a file: {}", path.display());
        }
        tokio::fs::remove_file(&path)
            .await
            .with_context(|| format!("failed to delete {}", path.display()))?;
        Ok(DeleteResult { path })
    }
}

fn sandbox_outer_wait_timeout(
    status: &SandboxCommandStatus,
    command_timeout: Duration,
    termination_timeout: Duration,
) -> Duration {
    if matches!(status, SandboxCommandStatus::Wrapped { .. }) {
        // The helper owns the authoritative command deadline. Leave enough
        // time for Job Object termination, exit confirmation, and the
        // elevated broker/runner result exchange so its staged diagnostic is
        // not replaced by a generic outer timeout.
        command_timeout
            .saturating_add(termination_timeout)
            .saturating_add(termination_timeout)
            .saturating_add(Duration::from_secs(20))
    } else {
        command_timeout
    }
}

async fn read_pipe_with_limit<R: AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: Option<usize>,
    limit_reached: CancellationToken,
    sink: Option<Arc<dyn BackgroundOutputSink>>,
    stream: OutputStream,
) -> (Vec<u8>, bool) {
    let mut output = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        tokio::select! {
            result = reader.read(&mut buf) => {
                match result {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Some(sink) = sink.as_ref() {
                            sink.push(stream, &buf[..n]);
                        }
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
            let marker = format!(
                "\n\n[output truncated by resource limit: {} bytes omitted]\n\n",
                bytes.len().saturating_sub(max)
            );
            let marker_bytes = marker.as_bytes();
            let head_len = max / 2;
            let tail_len = max.saturating_sub(head_len);
            let mut truncated = Vec::with_capacity(max.saturating_add(marker_bytes.len()));
            truncated.extend_from_slice(&bytes[..head_len]);
            truncated.extend_from_slice(marker_bytes);
            truncated.extend_from_slice(&bytes[bytes.len().saturating_sub(tail_len)..]);
            truncated
        }
        _ => bytes,
    }
}

fn sandbox_infrastructure_error(stderr: &[u8], expected_nonce: Option<&str>) -> Option<String> {
    let stderr = String::from_utf8_lossy(stderr);
    stderr.lines().find_map(|line| {
        let payload = line.strip_prefix(SANDBOX_ERROR_PREFIX)?;
        let envelope: serde_json::Value = serde_json::from_str(payload).ok()?;
        if envelope.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
            return None;
        }
        if envelope.get("nonce").and_then(serde_json::Value::as_str) != expected_nonce {
            return None;
        }
        envelope
            .get("message")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    })
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
    sandbox_error_nonce: Option<String>,
    stderr_observed: tokio::sync::Mutex<Vec<u8>>,
    execution_timeout: Duration,
    termination_timeout: Duration,
    /// Kept for the session's lifetime. The job terminates its members when the last handle
    /// closes, so releasing this before the session ends would kill the child.
    quota: tokio::sync::Mutex<Option<ProcessQuota>>,
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
        let mut observed = self.stderr_observed.lock().await;
        append_bounded_diagnostic(&mut observed, &buf);
        Ok(buf)
    }

    async fn close(&self) -> anyhow::Result<ExecResult> {
        {
            let mut stdin = self.stdin.lock().await;
            let _ = stdin.shutdown().await;
        }

        self.wait().await
    }

    async fn wait(&self) -> anyhow::Result<ExecResult> {
        let mut child_guard = self.child.lock().await;
        let mut child = child_guard.take();

        if let Some(ref mut child) = child {
            let cancel = self.cancel.clone();
            let cancel_token = self.cancel_token.clone();
            let wait_result = tokio::select! {
                result = child.wait() => result.map_err(anyhow::Error::from),
                _ = async {
                    if let Some(cancel) = cancel {
                        cancel.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    terminate_process(child, self.termination_timeout).await?;
                    Err(anyhow::anyhow!("stdio session cancelled during wait"))
                }
                _ = async {
                    if let Some(cancel) = cancel_token {
                        cancel.cancelled().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    terminate_process(child, self.termination_timeout).await?;
                    Err(anyhow::anyhow!("stdio session cancelled during wait"))
                }
                _ = tokio::time::sleep(self.execution_timeout) => {
                    terminate_process(child, self.termination_timeout).await?;
                    Err(anyhow::anyhow!(
                        "execution failed during wait: stdio session timed out after {:?}; process tree terminated",
                        self.execution_timeout
                    ))
                }
            };

            // End the entire job before returning or propagating a wait
            // error; otherwise a descendant can survive the session root.
            self.quota.lock().await.take();

            if let Some(ref request_id) = self.request_id {
                if let Some(ref env) = self.env {
                    env.unregister_process(request_id);
                }
            }

            let exit_status = wait_result?;
            if exit_status.code() == Some(SANDBOX_ERROR_EXIT_CODE) {
                let mut remaining = Vec::new();
                self.stderr.lock().await.read_to_end(&mut remaining).await?;
                let mut observed = self.stderr_observed.lock().await;
                append_bounded_diagnostic(&mut observed, &remaining);
                if let Some(message) =
                    sandbox_infrastructure_error(&observed, self.sandbox_error_nonce.as_deref())
                {
                    return Err(ExecutionFailure::without_os_error(
                        ExecutionStage::PrepareSandbox,
                        message,
                    )
                    .into());
                }
            }
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
        let termination = if let Some(mut child) = child_guard.take() {
            terminate_process(&mut child, self.termination_timeout).await
        } else {
            Ok(())
        };
        self.quota.lock().await.take();
        termination?;

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

fn append_bounded_diagnostic(buffer: &mut Vec<u8>, chunk: &[u8]) {
    const LIMIT: usize = 64 * 1024;
    buffer.extend_from_slice(chunk);
    if buffer.len() > LIMIT {
        buffer.drain(..buffer.len() - LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn sandbox_error_envelope_is_distinct_from_a_target_exit_code() {
        let marked = br#"ordinary stderr
OPENTOPIA_SANDBOX_ERROR {"version":1,"stage":"broker","nonce":"abc123","message":"policy unavailable"}
"#;
        assert_eq!(
            sandbox_infrastructure_error(marked, Some("abc123")).as_deref(),
            Some("policy unavailable")
        );
        assert_eq!(
            sandbox_infrastructure_error(marked, Some("wrong nonce")),
            None
        );
        assert_eq!(
            sandbox_infrastructure_error(b"target failed", Some("abc123")),
            None
        );
        assert_eq!(
            sandbox_infrastructure_error(
                br#"OPENTOPIA_SANDBOX_ERROR {"version":2,"nonce":"abc123","message":"future"}"#,
                Some("abc123")
            ),
            None
        );
    }

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

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_powershell_diagnostics_are_utf8() {
        let root = std::env::temp_dir().join(format!("opentopia-shell-utf8-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());
        let diagnostic = format!("{}{}{}{}", '\u{8bca}', '\u{65ad}', '\u{9519}', '\u{8bef}');
        let command = format!("Write-Error '{diagnostic}'");

        let exec = env
            .exec(
                ExecRequest::shell(command),
                ExecutionContext::with_timeout(Duration::from_secs(10)),
            )
            .await
            .expect("PowerShell returns a process result");
        assert!(!exec.success);
        let stderr = std::str::from_utf8(&exec.stderr).expect("stderr is UTF-8");
        assert!(stderr.contains(&diagnostic));

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
    async fn approved_external_write_path_does_not_authorize_siblings() {
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-lease-root-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-core-lease-outside-{id}"));
        std::fs::create_dir_all(&root).expect("create lease workspace");
        std::fs::create_dir_all(&outside).expect("create lease outside root");
        let approved = outside.join("approved.txt");
        let sibling = outside.join("sibling.txt");
        let mut config = LocalSandboxConfig::default();
        config.grant_write_path(approved.clone());
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);

        env.write_file(FileWriteRequest::new(&approved, b"allowed".to_vec()))
            .await
            .expect("write exact approved path");
        let error = env
            .write_file(FileWriteRequest::new(&sibling, b"blocked".to_vec()))
            .await
            .expect_err("sibling must remain outside the lease");

        assert!(error.to_string().contains("escapes workspace"));
        assert_eq!(std::fs::read_to_string(approved).unwrap(), "allowed");
        assert!(!sibling.exists());
        std::fs::remove_dir_all(root).expect("remove lease workspace");
        std::fs::remove_dir_all(outside).expect("remove lease outside root");
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
    async fn local_environment_windows_best_effort_unelevated_sandbox_executes() {
        if crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
            return;
        }
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let mut config = LocalSandboxConfig::best_effort();
        config.network = NetworkPolicy::Allow;
        config.windows_backend = crate::sandbox::WindowsSandboxBackend::Unelevated;
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);

        let exec = env
            .exec(
                ExecRequest::shell("Write-Output ok"),
                ExecutionContext::with_timeout(Duration::from_secs(45)),
            )
            .await
            .expect("OpenTopia Windows sandbox should run");

        assert!(
            exec.success,
            "sandboxed PowerShell failed: {}",
            String::from_utf8_lossy(&exec.stderr)
        );
        assert!(String::from_utf8_lossy(&exec.stdout).contains("ok"));
        let sandbox = exec.sandbox.expect("execution records sandbox metadata");
        assert_eq!(
            sandbox.permission_profile,
            "opentopia-windows-workspace-write-internet"
        );
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
    async fn local_environment_windows_restricted_token_denies_outside_write() {
        if crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
            return;
        }
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-sandbox-{id}"));
        let outside = std::env::current_dir()
            .expect("current directory")
            .parent()
            .expect("workspace parent")
            .join(format!("opentopia-core-outside-{id}.txt"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let mut config = LocalSandboxConfig::best_effort();
        config.network = NetworkPolicy::Allow;
        config.windows_backend = crate::sandbox::WindowsSandboxBackend::Unelevated;
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);
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
            .expect("OpenTopia Windows sandbox command should start");

        let outside_was_written = outside.exists();
        let command_succeeded = exec.success;
        std::fs::remove_dir_all(root).expect("remove temp workspace");
        let _ = std::fs::remove_file(outside);
        assert!(!outside_was_written, "sandbox wrote outside the workspace");
        assert!(
            !command_succeeded,
            "outside write should fail in the restricted-token backend"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_environment_windows_restricted_token_read_only_denies_workspace_write() {
        if crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
            return;
        }
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-readonly-{id}"));
        let sandbox_home = std::env::temp_dir().join(format!("opentopia-core-readonly-home-{id}"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let mut config = LocalSandboxConfig::best_effort().with_sandbox_mode(SandboxMode::ReadOnly);
        config.network = NetworkPolicy::Allow;
        config.windows_backend = crate::sandbox::WindowsSandboxBackend::Unelevated;
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
    async fn local_environment_windows_restricted_token_allows_additional_writable_root() {
        if crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
            return;
        }
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-root-{id}"));
        let extra = std::env::temp_dir().join(format!("opentopia-core-writable-{id}"));
        let sandbox_home = std::env::temp_dir().join(format!("opentopia-core-writable-home-{id}"));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        std::fs::create_dir_all(&extra).expect("create extra writable root");
        let mut config = LocalSandboxConfig::best_effort();
        config.network = NetworkPolicy::Allow;
        config.windows_backend = crate::sandbox::WindowsSandboxBackend::Unelevated;
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

    /// Allocates 256 MiB and reports which branch it took, so the same script can prove both
    /// that a quota bites and that the allocation is otherwise fine.
    #[cfg(windows)]
    const ALLOCATION_PROBE: &str = "$ErrorActionPreference='Stop'; try { $d = New-Object byte[] (256MB); $d[0]=1; Write-Output 'allocated' } catch { Write-Output 'allocation-failed' }";

    /// A quota must actually stop the command, not merely be configured.
    ///
    /// The script allocates well past the limit, so a working job object makes the
    /// allocation fail and the command exit non-zero. Without the job it would succeed.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_memory_quota_stops_an_over_allocating_command() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        // 256 MiB of allocation against a 64 MiB job limit.
        let exec = env
            .exec(
                ExecRequest::shell(ALLOCATION_PROBE),
                ExecutionContext::with_timeout(Duration::from_secs(60)).with_resource_limits(
                    ResourceLimit {
                        max_memory_bytes: Some(64 * 1024 * 1024),
                        ..Default::default()
                    },
                ),
            )
            .await
            .expect("exec should complete rather than error");

        // The exit code is not the signal here: a failed allocation surfaces as a
        // non-terminating PowerShell error, which still exits zero. What matters is that the
        // allocation did not succeed.
        let stdout = String::from_utf8_lossy(&exec.stdout);
        assert!(
            stdout.contains("allocation-failed"),
            "the job memory limit did not stop the allocation; stdout={stdout} stderr={}",
            String::from_utf8_lossy(&exec.stderr)
        );
        assert!(
            !stdout.contains("allocated"),
            "the command allocated past its quota; stdout={stdout}"
        );
        std::fs::remove_dir_all(root).expect("remove temp workspace");
    }

    /// The same command must succeed without a quota, so the assertion above is attributable
    /// to the job object rather than to the script being broken.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_allocation_succeeds_without_a_quota() {
        let root =
            std::env::temp_dir().join(format!("opentopia-core-execution-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp workspace");
        let env = LocalExecutionEnvironment::new(root.clone());

        let exec = env
            .exec(
                ExecRequest::shell(ALLOCATION_PROBE),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("exec should complete");

        assert!(
            String::from_utf8_lossy(&exec.stdout).contains("allocated"),
            "baseline allocation failed, so the quota test proves nothing; stderr={}",
            String::from_utf8_lossy(&exec.stderr)
        );
        std::fs::remove_dir_all(root).expect("remove temp workspace");
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
