mod local_environment;
mod stdio_session;

pub use local_environment::LocalExecutionEnvironment;
pub use stdio_session::LocalStdioSession;

use crate::execution_authorization::ProcessLifetime;
pub use crate::execution_spec::{
    shell_command_compatibility_error, EnvironmentPolicy, ExecRequest, ExecutionFailure,
    ExecutionRequirements, ExecutionSpec, ExecutionStage, LifecyclePolicy, RuntimeRequirements,
    ShellCompatibilityError, ShellDialect, StdioPolicy,
};
use crate::sandbox::{ExecutionEnvironmentKind, NetworkPolicy, SandboxCommandStatus, SandboxMode};
use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{LocalSandboxConfig, OsSandboxPlatform};
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
    async fn parent_traversal_is_blocked_but_absolute_host_reads_remain_available() {
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
            .expect("ordinary absolute host read must be allowed");
        assert_eq!(absolute_parent.bytes, b"allowed");

        let parent_cwd = env
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
            .expect("absolute host cwd must be allowed by the execution boundary");
        assert!(parent_cwd.success);

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
    async fn local_environment_windows_dedicated_user_read_command_matrix() {
        if !crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
            return;
        }
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-read-matrix-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-core-read-inputs-{id}"));
        let sandbox_home = std::env::temp_dir().join(format!("opentopia-core-read-home-{id}"));
        std::fs::create_dir_all(&root).expect("create read-matrix workspace");
        std::fs::create_dir_all(&outside).expect("create read-matrix input directory");
        let text = outside.join("input.txt");
        let csv = outside.join("orders.csv");
        std::fs::write(&text, "external-readable").expect("write text fixture");
        std::fs::write(&csv, "id,status\n42,open\n").expect("write csv fixture");

        let mut config = LocalSandboxConfig::enforce();
        config.network = NetworkPolicy::Deny;
        config.sandbox_home = Some(sandbox_home.clone());
        config.grant_read_path(outside.clone());
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config);
        let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
        let commands = [
            (
                format!(
                    "Get-ChildItem -LiteralPath '{}' | Select-Object -ExpandProperty Name",
                    quote(&outside)
                ),
                "orders.csv",
            ),
            (
                format!(
                    "Import-Csv -LiteralPath '{}' | Select-Object -ExpandProperty id",
                    quote(&csv)
                ),
                "42",
            ),
            (
                format!(
                    "[System.IO.File]::ReadAllBytes('{}').Length",
                    quote(&text)
                ),
                "17",
            ),
            (
                format!(
                    "$reader = [System.IO.StreamReader]::new('{}'); try {{ $reader.ReadToEnd() }} finally {{ $reader.Dispose() }}",
                    quote(&text)
                ),
                "external-readable",
            ),
        ];

        for (command, expected) in commands {
            let output = env
                .exec(
                    ExecRequest::shell(command.clone()),
                    ExecutionContext::with_timeout(Duration::from_secs(60)),
                )
                .await
                .unwrap_or_else(|error| panic!("read command failed to start: {command}: {error}"));
            assert!(
                output.success && String::from_utf8_lossy(&output.stdout).contains(expected),
                "sandboxed read command failed: {command}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }

        std::fs::remove_dir_all(root).expect("remove read-matrix workspace");
        std::fs::remove_dir_all(outside).expect("remove read-matrix input directory");
        let _ = std::fs::remove_dir_all(sandbox_home);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn local_environment_windows_dedicated_user_command_matrix() {
        if !crate::sandbox::dedicated_user_credentials_are_installed_for_tests() {
            return;
        }
        let id = Uuid::new_v4();
        let root = std::env::temp_dir().join(format!("opentopia-core-dedicated-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-core-host-read-{id}"));
        let sandbox_home = std::env::temp_dir().join(format!("opentopia-core-home-{id}"));
        std::fs::create_dir_all(&root).expect("create dedicated workspace");
        std::fs::write(
            root.join("package.json"),
            r#"{"packageManager":"pnpm@10.30.0"}"#,
        )
        .expect("declare managed pnpm runtime");
        std::fs::create_dir_all(&outside).expect("create host read fixture");
        let external_read = outside.join("input.txt");
        let external_write = outside.join("blocked.txt");
        std::fs::write(&external_read, "external-readable").expect("write host read fixture");

        let mut config = LocalSandboxConfig::enforce();
        config.network = NetworkPolicy::Allow;
        config.sandbox_home = Some(sandbox_home.clone());
        config.grant_read_path(external_read.clone());
        let env = LocalExecutionEnvironment::with_sandbox_config(root.clone(), config.clone());
        let quote = |path: &Path| path.to_string_lossy().replace('\'', "''");

        let read = env
            .exec(
                ExecRequest::shell(format!(
                    "Get-Content -LiteralPath '{}'",
                    quote(&external_read)
                )),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("dedicated-user external read command should start");
        assert!(
            read.success && String::from_utf8_lossy(&read.stdout).contains("external-readable"),
            "dedicated-user external read failed: {}",
            String::from_utf8_lossy(&read.stderr)
        );

        let nested_runtime = env
            .exec(
                ExecRequest::shell("cargo --version"),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("dedicated-user nested PATH runtime command should start");
        assert!(
            nested_runtime.success
                && String::from_utf8_lossy(&nested_runtime.stdout).contains("cargo"),
            "nested PATH runtime failed: {}",
            String::from_utf8_lossy(&nested_runtime.stderr)
        );

        let managed_pnpm = env
            .exec(
                ExecRequest::shell("pnpm --version"),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("managed pnpm command should start in dedicated-user sandbox");
        assert!(
            managed_pnpm.success
                && String::from_utf8_lossy(&managed_pnpm.stdout).contains("10.30.0"),
            "managed pnpm failed inside dedicated-user sandbox\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&managed_pnpm.stdout),
            String::from_utf8_lossy(&managed_pnpm.stderr)
        );

        // Node-based tools normally launch compilers, workers, or another Node
        // process. A top-level `node --version` cannot detect a sandbox/job
        // policy that permits the parent but rejects its descendants.
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok()
        {
            let child_probe = root.join("node-child-process-canary.cjs");
            std::fs::write(
                &child_probe,
                r#"const { spawnSync } = require('node:child_process');
const child = spawnSync(process.execPath, ['-e', 'process.stdout.write("child-ok")'], { encoding: 'utf8' });
if (child.error) throw child.error;
process.stdout.write(child.stdout || '');
process.stderr.write(child.stderr || '');
process.exit(child.status ?? 1);
"#,
            )
            .expect("write Node child-process canary");
            let nested_child = env
                .exec(
                    ExecRequest::shell(format!("node '{}'", quote(&child_probe))),
                    ExecutionContext::with_timeout(Duration::from_secs(60)),
                )
                .await
                .expect("dedicated-user Node child-process canary should start");
            assert!(
                nested_child.success
                    && String::from_utf8_lossy(&nested_child.stdout).contains("child-ok"),
                "Node child process failed inside the sandbox\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&nested_child.stdout),
                String::from_utf8_lossy(&nested_child.stderr)
            );
        }

        let inside = root.join("inside.txt");
        let write_inside = env
            .exec(
                ExecRequest::shell(format!(
                    "Set-Content -LiteralPath '{}' -Value allowed",
                    quote(&inside)
                )),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("dedicated-user workspace write command should start");
        assert!(write_inside.success, "workspace write should succeed");
        assert!(inside.exists());

        let other_root = std::env::temp_dir().join(format!("opentopia-core-other-{id}"));
        std::fs::create_dir_all(&other_root).expect("create second workspace");
        let other_env =
            LocalExecutionEnvironment::with_sandbox_config(other_root.clone(), config.clone());
        let cross_workspace = other_env
            .exec(
                ExecRequest::shell(format!(
                    "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value escaped",
                    quote(&inside)
                )),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("cross-workspace write probe should start");
        assert!(
            !cross_workspace.success,
            "dedicated identity escaped into a previously granted workspace"
        );

        let write_outside = env
            .exec(
                ExecRequest::shell(format!(
                    "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value blocked",
                    quote(&external_write)
                )),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("dedicated-user outside write command should start");
        assert!(
            !write_outside.success,
            "outside write unexpectedly succeeded"
        );
        assert!(!external_write.exists());

        let read_only = LocalExecutionEnvironment::with_sandbox_config(
            root.clone(),
            config.with_sandbox_mode(SandboxMode::ReadOnly),
        );
        let read_only_target = root.join("read-only-blocked.txt");
        let read_only_write = read_only
            .exec(
                ExecRequest::shell(format!(
                    "$ErrorActionPreference='Stop'; Set-Content -LiteralPath '{}' -Value blocked",
                    quote(&read_only_target)
                )),
                ExecutionContext::with_timeout(Duration::from_secs(60)),
            )
            .await
            .expect("read-only dedicated-user command should start");
        assert!(
            !read_only_write.success,
            "read-only write unexpectedly succeeded"
        );
        assert!(!read_only_target.exists());

        std::fs::remove_dir_all(root).expect("remove dedicated workspace");
        std::fs::remove_dir_all(other_root).expect("remove second workspace");
        std::fs::remove_dir_all(outside).expect("remove host read fixture");
        let _ = std::fs::remove_dir_all(sandbox_home);
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
