//! Local process execution, sandbox preparation, and filesystem operations.

use super::stdio_session::LocalStdioSession;
use super::{
    read_pipe_with_limit, sandbox_infrastructure_error, sandbox_outer_wait_timeout,
    truncate_output_vec, DeleteResult, ExecResult, ExecutionContext, ExecutionEnvironment,
    ExecutionSandboxMetadata, FileDeleteRequest, FileReadRequest, FileReadResult, FileWriteRequest,
    OutputStream, StdioSession, WriteResult, SANDBOX_ERROR_EXIT_CODE, SANDBOX_ERROR_NONCE_ENV,
};
use crate::execution_runtime::{
    configure_command_environment, configure_stdio, environment_keys, resolve_runtime,
};
use crate::execution_spec::{ExecRequest, ExecutionFailure, ExecutionStage};
use crate::policy::ApprovalRequired;
use crate::process_quota::ProcessQuota;
use crate::process_supervisor::{spawn_process, terminate_process};
use crate::sandbox::{
    build_local_sandbox_command_with_options, is_protected_metadata_path,
    sandbox_permission_profile, ExecutionEnvironmentKind, LocalSandboxConfig, NetworkPolicy,
    OsSandboxPlatform, SandboxBackendCapabilities, SandboxCommandStatus, SandboxLaunchOptions,
    SandboxMode, SandboxPreparationPlan, WindowsSandboxBackend,
};
use crate::workspace_execution_capsule::WorkspaceExecutionCapsule;
use anyhow::Context;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const TRANSIENT_WRITE_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(25),
    Duration::from_millis(75),
    Duration::from_millis(200),
];

type SandboxPreparationLock = tokio::sync::Mutex<()>;

// Tool contexts are rebuilt whenever an execution grant changes. Preparation
// state therefore belongs to the local executor process, not to one context.
// Weak single-flight locks avoid retaining one mutex for every historical ACL
// scope while completed keys remain reusable across tools and conversations.
static PREPARED_SANDBOX_PHASES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static SANDBOX_PREPARATION_LOCKS: OnceLock<Mutex<HashMap<String, Weak<SandboxPreparationLock>>>> =
    OnceLock::new();

fn prepared_sandbox_phases() -> &'static Mutex<HashSet<String>> {
    PREPARED_SANDBOX_PHASES.get_or_init(|| Mutex::new(HashSet::new()))
}

fn sandbox_preparation_locks() -> &'static Mutex<HashMap<String, Weak<SandboxPreparationLock>>> {
    SANDBOX_PREPARATION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_transient_write_error(error: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        // Sharing violation, lock violation, and a file with a mapped section
        // open. Editors, indexers, antivirus, and running binaries can hold
        // these conditions briefly; retrying other I/O failures hides bugs.
        matches!(error.raw_os_error(), Some(32 | 33 | 1224))
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

async fn read_write_retry_snapshot(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

async fn write_file_with_transient_retry(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    let first_error = match tokio::fs::write(path, contents).await {
        Ok(()) => return Ok(()),
        Err(error) if is_transient_write_error(&error) => error,
        Err(error) => return Err(error.into()),
    };
    let expected = read_write_retry_snapshot(path).await.with_context(|| {
        format!(
            "failed to revalidate {} after a transient write conflict",
            path.display()
        )
    })?;
    let mut last_error = first_error;

    for delay in TRANSIENT_WRITE_RETRY_DELAYS {
        tokio::time::sleep(delay).await;
        let current = read_write_retry_snapshot(path).await.with_context(|| {
            format!(
                "failed to revalidate {} before retrying a transient write conflict",
                path.display()
            )
        })?;
        anyhow::ensure!(
            current == expected,
            "file changed while waiting to retry a transient write conflict: {}; reread the latest file and retry",
            path.display()
        );
        match tokio::fs::write(path, contents).await {
            Ok(()) => return Ok(()),
            Err(error) if is_transient_write_error(&error) => last_error = error,
            Err(error) => return Err(error.into()),
        }
    }

    Err(last_error).with_context(|| {
        format!(
            "transient write conflict persisted after {} retries for {}",
            TRANSIENT_WRITE_RETRY_DELAYS.len(),
            path.display()
        )
    })
}

#[derive(Debug, Clone)]
pub struct LocalExecutionEnvironment {
    id: String,
    workspace_root: PathBuf,
    sandbox_config: LocalSandboxConfig,
    execution_capsule: Arc<WorkspaceExecutionCapsule>,
    pub(super) running: Arc<Mutex<HashMap<String, CancellationToken>>>,
}

impl LocalExecutionEnvironment {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self::build(
            "local".to_string(),
            workspace_root.into(),
            LocalSandboxConfig::default(),
        )
    }

    pub fn with_id(id: impl Into<String>, workspace_root: impl Into<PathBuf>) -> Self {
        Self::build(
            id.into(),
            workspace_root.into(),
            LocalSandboxConfig::default(),
        )
    }

    pub fn with_sandbox_config(
        workspace_root: impl Into<PathBuf>,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        Self::build("local".to_string(), workspace_root.into(), sandbox_config)
    }

    pub fn with_id_and_sandbox_config(
        id: impl Into<String>,
        workspace_root: impl Into<PathBuf>,
        sandbox_config: LocalSandboxConfig,
    ) -> Self {
        Self::build(id.into(), workspace_root.into(), sandbox_config)
    }

    fn build(id: String, workspace_root: PathBuf, sandbox_config: LocalSandboxConfig) -> Self {
        let execution_capsule = Arc::new(WorkspaceExecutionCapsule::discover(&workspace_root));
        for issue in execution_capsule.issues() {
            tracing::warn!(
                workspace = %workspace_root.display(),
                capability = issue.capability,
                reason = %issue.reason,
                "workspace execution capability is unavailable"
            );
        }
        tracing::debug!(
            workspace = %workspace_root.display(),
            capsule = execution_capsule.fingerprint(),
            "workspace execution capsule resolved"
        );
        Self {
            id,
            workspace_root,
            sandbox_config,
            execution_capsule,
            running: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn sandbox_config(&self) -> &LocalSandboxConfig {
        &self.sandbox_config
    }

    fn persistent_stdio_sandbox_config(&self) -> anyhow::Result<LocalSandboxConfig> {
        let mut config = self.sandbox_config.clone();
        if OsSandboxPlatform::current() == OsSandboxPlatform::Windows
            && config.is_enabled()
            && config.effective_windows_backend() == WindowsSandboxBackend::DedicatedUser
        {
            anyhow::ensure!(
                config.network != NetworkPolicy::Deny,
                "persistent stdio cannot use the Windows dedicated-user backend, and the streaming restricted-token backend cannot authoritatively enforce offline networking"
            );

            // CreateProcessWithLogonW cannot inherit the caller's stdio handles. The
            // dedicated-user helper therefore captures one-shot output in files and
            // attaches NUL as stdin, which is incompatible with MCP and other
            // bidirectional persistent services. Keep this downgrade local to the
            // stdio process: the WRITE_RESTRICTED backend preserves workspace write
            // containment and native stdio while the session's online network policy
            // remains unchanged. One-shot commands continue to use strict isolation.
            config.windows_backend = WindowsSandboxBackend::Unelevated;
            tracing::warn!(
                workspace = %self.workspace_root.display(),
                "using the Windows streaming restricted-token sandbox for persistent stdio"
            );
        }
        Ok(config)
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
        // Read-only and workspace-write describe mutation authority. They both
        // permit host reads; command sandboxes receive the declared read paths
        // through ExecutionGrant so a different OS identity can access them.
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

    pub(super) fn unregister_process(&self, request_id: &str) {
        self.running.lock().unwrap().remove(request_id);
    }

    async fn prepare_sandbox_phase(
        &self,
        preparation: Option<&SandboxPreparationPlan>,
        cwd: &Path,
        startup_timeout: Duration,
    ) -> anyhow::Result<()> {
        let Some(preparation) = preparation else {
            return Ok(());
        };
        let cache_key = format!("{}\0{}", preparation.program, preparation.key);
        if prepared_sandbox_phases()
            .lock()
            .unwrap()
            .contains(&cache_key)
        {
            return Ok(());
        }
        let scope_lock = {
            let mut locks = sandbox_preparation_locks().lock().unwrap();
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(&cache_key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(SandboxPreparationLock::new(()));
                locks.insert(cache_key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let _scope_guard = scope_lock.lock().await;
        if prepared_sandbox_phases()
            .lock()
            .unwrap()
            .contains(&cache_key)
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
        prepared_sandbox_phases().lock().unwrap().insert(cache_key);
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

fn sandbox_command_timeout_ms(context: &ExecutionContext) -> u64 {
    context.timeout.as_millis().min(u128::from(u64::MAX)) as u64
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

        let runtime = resolve_runtime(
            &request,
            &cwd,
            &self.workspace_root,
            &self.sandbox_config,
            &self.execution_capsule,
        )?;
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
                persistent_stdio: false,
                runtime_read_roots: runtime.read_roots.clone(),
                managed_runtime_roots: runtime.managed_runtime_roots.clone(),
                environment_keys: environment_keys(&runtime),
                additional_denied_read_paths: request.requirements.deny_read_paths.clone(),
                additional_protected_paths: request.requirements.deny_write_paths.clone(),
                // Keep the inner sandbox lifecycle aligned with the tool's command
                // timeout. Otherwise the wrapper falls back to its 30-second default
                // and can terminate a command even though the caller granted longer.
                timeout_ms: Some(sandbox_command_timeout_ms(&context)),
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

        self.prepare_sandbox_phase(
            command_plan.baseline_preparation.as_ref(),
            &cwd,
            context.startup_timeout,
        )
        .await?;
        self.prepare_sandbox_phase(
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

        let sandbox_config = self.persistent_stdio_sandbox_config()?;

        let runtime = resolve_runtime(
            &request,
            &cwd,
            &self.workspace_root,
            &sandbox_config,
            &self.execution_capsule,
        )?;
        let program = runtime.program.to_string_lossy().into_owned();

        let command_plan = build_local_sandbox_command_with_options(
            &program,
            &request.args,
            &cwd,
            &self.workspace_root,
            &sandbox_config,
            &SandboxLaunchOptions {
                interactive: false,
                persistent_stdio: true,
                runtime_read_roots: runtime.read_roots.clone(),
                managed_runtime_roots: runtime.managed_runtime_roots.clone(),
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
                &sandbox_config,
            ),
            sandbox_mode: sandbox_config.sandbox_mode,
            network: sandbox_config.network,
        });
        let outer_wait_timeout = sandbox_outer_wait_timeout(
            &command_plan.status,
            context.timeout,
            context.termination_timeout,
        );
        self.prepare_sandbox_phase(
            command_plan.baseline_preparation.as_ref(),
            &cwd,
            context.startup_timeout,
        )
        .await?;
        self.prepare_sandbox_phase(
            command_plan.preparation.as_ref(),
            &cwd,
            context.startup_timeout,
        )
        .await?;
        let mut process = Command::new(&command_plan.program);
        configure_command_environment(&mut process, &request, &runtime, &sandbox_config);
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
        write_file_with_transient_retry(&path, &request.contents)
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn one_shot_sandbox_preserves_the_requested_command_timeout() {
        let context = ExecutionContext::with_timeout(Duration::from_secs(180));

        assert_eq!(sandbox_command_timeout_ms(&context), 180_000);
    }

    #[test]
    fn persistent_stdio_uses_streaming_windows_backend_for_online_sessions() {
        let mut config = LocalSandboxConfig::enforce();
        config.network = NetworkPolicy::Allow;
        config.windows_backend = WindowsSandboxBackend::DedicatedUser;
        let environment = LocalExecutionEnvironment::with_sandbox_config("C:\\", config);

        let effective = environment
            .persistent_stdio_sandbox_config()
            .expect("online persistent stdio should have a streaming sandbox backend");

        assert_eq!(effective.mode, crate::sandbox::OsSandboxMode::Enforce);
        assert_eq!(effective.windows_backend, WindowsSandboxBackend::Unelevated);
        assert_eq!(effective.network, NetworkPolicy::Allow);
    }

    #[test]
    fn persistent_stdio_does_not_weaken_offline_network_policy() {
        let mut config = LocalSandboxConfig::enforce();
        config.network = NetworkPolicy::Deny;
        config.windows_backend = WindowsSandboxBackend::DedicatedUser;
        let environment = LocalExecutionEnvironment::with_sandbox_config("C:\\", config);

        let error = environment
            .persistent_stdio_sandbox_config()
            .expect_err("offline persistent stdio must fail closed");

        assert!(error
            .to_string()
            .contains("cannot authoritatively enforce offline networking"));
    }

    #[tokio::test]
    async fn sandbox_preparation_cache_is_shared_across_environment_rebuilds() {
        let key = format!("shared-preparation-test-{}", uuid::Uuid::new_v4());
        let first = SandboxPreparationPlan {
            key: key.clone(),
            program: "cmd.exe".to_string(),
            args: vec!["/d".to_string(), "/c".to_string(), "exit 0".to_string()],
            env: Vec::new(),
        };
        LocalExecutionEnvironment::new("C:\\")
            .prepare_sandbox_phase(Some(&first), Path::new("C:\\"), Duration::from_secs(1))
            .await
            .expect("first environment prepares the shared phase");

        let would_fail_if_started = SandboxPreparationPlan {
            key,
            program: "cmd.exe".to_string(),
            args: vec!["/d".to_string(), "/c".to_string(), "exit 1".to_string()],
            env: Vec::new(),
        };
        LocalExecutionEnvironment::new("C:\\")
            .prepare_sandbox_phase(
                Some(&would_fail_if_started),
                Path::new("C:\\"),
                Duration::from_secs(1),
            )
            .await
            .expect("rebuilt environment reuses the process-wide preparation");
    }
}
