//! Long-running work that outlives the tool call that started it.
//!
//! A blocking `shell` call ties a command's runtime to a model round: the model
//! can do nothing else while it waits, the output only appears at the end, and a
//! cancelled turn throws the work away. That is acceptable for a `git status` and
//! wrong for an install, a build, or a download.
//!
//! This registry lets commands and bounded non-process tasks run detached without
//! creating separate lifecycle managers. Commands keep the ordinary sandbox, quota,
//! and cancellation semantics of [`ExecutionEnvironment`]; what changes is who waits.
//! Output accumulates as it arrives, the model can pull it at any time, and completion
//! is pushed into the next model round rather than polled for.

use crate::execution::{
    BackgroundOutputSink, ExecRequest, ExecutionContext, ExecutionEnvironment,
    ExecutionSandboxMetadata, OutputStream, StdioSession,
};
use crate::policy::approval_required;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Output kept per stream for one job before the oldest bytes are dropped.
///
/// The cap protects the context window, not the disk: a command that prints a
/// megabyte a second must not be able to push the whole thing at the model.
const DEFAULT_MAX_BUFFERED_BYTES: usize = 64 * 1024;
/// Concurrent background jobs one agent may hold.
const DEFAULT_MAX_JOBS_PER_AGENT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundJobStatus {
    Running,
    Exited,
    Failed,
    Cancelled,
    TimedOut,
}

impl BackgroundJobStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }
}

/// Identity of the agent that owns a job, so one agent cannot read another's work.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackgroundScope {
    pub thread_id: Uuid,
    pub agent_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundJobSnapshot {
    pub job_id: Uuid,
    pub agent_path: String,
    pub command: String,
    /// Interactive jobs retain stdin and can receive input until they exit.
    pub interactive: bool,
    pub status: BackgroundJobStatus,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// Typed authorization boundary preserved across detached execution.
    pub approval_required: Option<String>,
    /// True when the execution backend had to truncate process output.
    pub truncated: bool,
    /// Sandbox details captured when the process exits.
    pub sandbox: Option<ExecutionSandboxMetadata>,
    /// Bytes produced in total, including any the buffer had to drop.
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub dropped_bytes: usize,
    /// Output the agent has not read yet.
    pub unread_bytes: usize,
}

/// Output handed to the model, together with what it cost to keep it bounded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundOutputChunk {
    pub job: BackgroundJobSnapshot,
    pub stdout: String,
    pub stderr: String,
    pub dropped_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct BackgroundRegistryConfig {
    pub max_buffered_bytes: usize,
    pub max_jobs_per_agent: usize,
}

impl Default for BackgroundRegistryConfig {
    fn default() -> Self {
        Self {
            max_buffered_bytes: DEFAULT_MAX_BUFFERED_BYTES,
            max_jobs_per_agent: DEFAULT_MAX_JOBS_PER_AGENT,
        }
    }
}

#[derive(Debug, Default)]
struct StreamBuffer {
    /// Output produced but not yet handed to the model.
    unread: Vec<u8>,
    total_bytes: usize,
    /// Bytes omitted from all chunks over the lifetime of this stream.
    dropped_bytes: usize,
    /// Bytes omitted from the chunk that has not yet been read.
    unread_dropped_bytes: usize,
}

impl StreamBuffer {
    fn push(&mut self, chunk: &[u8], max_bytes: usize) {
        self.total_bytes = self.total_bytes.saturating_add(chunk.len());
        self.unread.extend_from_slice(chunk);
        if self.unread.len() > max_bytes {
            // Keep both the invocation/prologue and the newest outcome. Repeated
            // pushes preserve the original head while rotating the tail.
            let original_len = self.unread.len();
            let head_len = max_bytes / 2;
            let tail_len = max_bytes.saturating_sub(head_len);
            let mut bounded = Vec::with_capacity(max_bytes);
            bounded.extend_from_slice(&self.unread[..head_len]);
            bounded.extend_from_slice(&self.unread[original_len - tail_len..]);
            self.unread = bounded;
            let excess = original_len - max_bytes;
            self.dropped_bytes = self.dropped_bytes.saturating_add(excess);
            self.unread_dropped_bytes = self.unread_dropped_bytes.saturating_add(excess);
        }
    }

    fn peek(&self) -> String {
        String::from_utf8_lossy(&self.unread).into_owned()
    }

    fn clear(&mut self) {
        self.unread.clear();
        self.unread_dropped_bytes = 0;
    }
}

#[derive(Clone)]
struct InteractiveSession {
    inner: Arc<dyn StdioSession>,
}

impl std::fmt::Debug for InteractiveSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InteractiveSession")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct JobState {
    status: BackgroundJobStatus,
    exit_code: Option<i32>,
    success: bool,
    finished_at: Option<DateTime<Utc>>,
    error: Option<String>,
    approval_required: Option<String>,
    truncated: bool,
    sandbox: Option<ExecutionSandboxMetadata>,
    stdout: StreamBuffer,
    stderr: StreamBuffer,
    /// Set once the model has been told this job reached a terminal state.
    completion_reported: bool,
}

#[derive(Debug)]
struct Job {
    id: Uuid,
    scope: BackgroundScope,
    command: String,
    started_at: DateTime<Utc>,
    cancel: CancellationToken,
    finished: CancellationToken,
    max_buffered_bytes: usize,
    session: Option<InteractiveSession>,
    state: Mutex<JobState>,
}

impl Job {
    fn lock(&self) -> std::sync::MutexGuard<'_, JobState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn snapshot(&self) -> BackgroundJobSnapshot {
        let state = self.lock();
        BackgroundJobSnapshot {
            job_id: self.id,
            agent_path: self.scope.agent_path.clone(),
            command: self.command.clone(),
            interactive: self.session.is_some(),
            status: state.status,
            exit_code: state.exit_code,
            success: state.success,
            started_at: self.started_at,
            finished_at: state.finished_at,
            error: state.error.clone(),
            approval_required: state.approval_required.clone(),
            truncated: state.truncated,
            sandbox: state.sandbox.clone(),
            stdout_bytes: state.stdout.total_bytes,
            stderr_bytes: state.stderr.total_bytes,
            dropped_bytes: state.stdout.dropped_bytes + state.stderr.dropped_bytes,
            unread_bytes: state.stdout.unread.len() + state.stderr.unread.len(),
        }
    }
}

#[derive(Debug)]
struct JobSink {
    job: Arc<Job>,
}

impl BackgroundOutputSink for JobSink {
    fn push(&self, stream: OutputStream, chunk: &[u8]) {
        let max = self.job.max_buffered_bytes;
        let mut state = self.job.lock();
        match stream {
            OutputStream::Stdout => state.stdout.push(chunk, max),
            OutputStream::Stderr => state.stderr.push(chunk, max),
        }
    }
}

#[derive(Debug)]
struct RegistryInner {
    config: BackgroundRegistryConfig,
    jobs: Mutex<HashMap<Uuid, Arc<Job>>>,
}

/// Handle to the background jobs of one runtime. Cloning shares the registry.
#[derive(Debug, Clone)]
pub struct BackgroundProcessRegistry {
    inner: Arc<RegistryInner>,
}

impl Default for BackgroundProcessRegistry {
    fn default() -> Self {
        Self::new(BackgroundRegistryConfig::default())
    }
}

pub struct BackgroundSpawnRequest {
    pub scope: BackgroundScope,
    pub command: String,
    pub request: ExecRequest,
    pub context: ExecutionContext,
}

pub struct BackgroundSessionSpawnRequest {
    pub scope: BackgroundScope,
    pub command: String,
    pub request: ExecRequest,
    pub context: ExecutionContext,
}

impl BackgroundProcessRegistry {
    pub fn new(config: BackgroundRegistryConfig) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                config,
                jobs: Mutex::new(HashMap::new()),
            }),
        }
    }

    fn jobs(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<Job>>> {
        self.inner
            .jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn visible(&self, scope: &BackgroundScope, job_id: Uuid) -> anyhow::Result<Arc<Job>> {
        let job = self
            .jobs()
            .get(&job_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("background job not found: {job_id}"))?;
        if &job.scope != scope {
            anyhow::bail!("background job {job_id} belongs to another agent");
        }
        Ok(job)
    }

    fn register_detached_job(
        &self,
        scope: BackgroundScope,
        label: String,
    ) -> anyhow::Result<(Arc<Job>, CancellationToken, CancellationToken)> {
        let running = self
            .jobs()
            .values()
            .filter(|job| job.scope == scope && !job.lock().status.is_terminal())
            .count();
        if running >= self.inner.config.max_jobs_per_agent {
            anyhow::bail!(
                "this agent already has {running} background jobs running (maximum {}); wait for one to finish or stop it first",
                self.inner.config.max_jobs_per_agent
            );
        }

        let cancel = CancellationToken::new();
        let finished = CancellationToken::new();
        let job = Arc::new(Job {
            id: Uuid::new_v4(),
            scope,
            command: label,
            started_at: Utc::now(),
            cancel: cancel.clone(),
            finished: finished.clone(),
            max_buffered_bytes: self.inner.config.max_buffered_bytes,
            session: None,
            state: Mutex::new(JobState {
                status: BackgroundJobStatus::Running,
                exit_code: None,
                success: false,
                finished_at: None,
                error: None,
                approval_required: None,
                truncated: false,
                sandbox: None,
                stdout: StreamBuffer::default(),
                stderr: StreamBuffer::default(),
                completion_reported: false,
            }),
        });
        self.jobs().insert(job.id, job.clone());
        Ok((job, cancel, finished))
    }

    fn link_turn_cancellation(
        turn_cancel: Option<CancellationToken>,
        job_cancel: CancellationToken,
        job_finished: CancellationToken,
    ) {
        if let Some(turn_cancel) = turn_cancel {
            tokio::spawn(async move {
                tokio::select! {
                    _ = turn_cancel.cancelled() => job_cancel.cancel(),
                    _ = job_finished.cancelled() => {}
                }
            });
        }
    }

    /// Starts a command and returns as soon as it is running.
    ///
    /// The command runs through the caller's [`ExecutionEnvironment`], so sandbox,
    /// quota, and environment scrubbing are identical to a foreground call.
    pub fn spawn(
        &self,
        environment: Arc<dyn ExecutionEnvironment>,
        spawn: BackgroundSpawnRequest,
    ) -> anyhow::Result<BackgroundJobSnapshot> {
        let BackgroundSpawnRequest {
            scope,
            command,
            request,
            mut context,
        } = spawn;

        let (job, cancel, finished) = self.register_detached_job(scope, command)?;

        // The job's own token stops just this command. A turn token, when the caller
        // supplied one, still stops everything at once: a user pressing stop should not
        // leave a download running with nobody left to read it.
        Self::link_turn_cancellation(context.cancel.take(), cancel.clone(), finished.clone());
        context.cancel = Some(cancel.clone());
        context.output_sink = Some(Arc::new(JobSink { job: job.clone() }));

        let snapshot = job.snapshot();

        tokio::spawn(async move {
            let outcome = environment.exec(request, context).await;
            let mut state = job.lock();
            state.finished_at = Some(Utc::now());
            match outcome {
                // The command ran to completion. A non-zero exit is its own answer, not
                // a runtime failure, so it stays an ordinary exit with the code attached.
                Ok(result) => {
                    state.status = BackgroundJobStatus::Exited;
                    state.exit_code = result.exit_code;
                    state.success = result.success;
                    state.truncated = result.truncated;
                    state.sandbox = result.sandbox;
                    if result.truncated {
                        state.error = Some(
                            "output exceeded the resource limit and was truncated".to_string(),
                        );
                    }
                }
                // Cancellation and timeout surface as errors from the execution
                // environment, so they are separated from a genuine spawn failure here.
                Err(error) => {
                    state.approval_required =
                        approval_required(&error).map(|required| required.reason().to_string());
                    let message = error.to_string();
                    state.status = if job.cancel.is_cancelled() || message.contains("cancelled") {
                        BackgroundJobStatus::Cancelled
                    } else if message.contains("timed out") {
                        BackgroundJobStatus::TimedOut
                    } else {
                        BackgroundJobStatus::Failed
                    };
                    state.error = Some(message);
                }
            }
            drop(state);
            finished.cancel();
        });

        Ok(snapshot)
    }

    /// Runs a non-process operation through the same bounded job lifecycle used
    /// by commands. This keeps downloads and opt-in remote work from requiring a
    /// second scheduler or a second set of read/stop/completion tools.
    pub fn spawn_task<F>(
        &self,
        scope: BackgroundScope,
        label: String,
        turn_cancel: Option<CancellationToken>,
        task: F,
    ) -> anyhow::Result<BackgroundJobSnapshot>
    where
        F: Future<Output = anyhow::Result<String>> + Send + 'static,
    {
        let (job, cancel, finished) = self.register_detached_job(scope, label)?;
        Self::link_turn_cancellation(turn_cancel, cancel.clone(), finished.clone());

        let snapshot = job.snapshot();
        tokio::spawn(async move {
            let outcome = tokio::select! {
                outcome = task => Some(outcome),
                _ = cancel.cancelled() => None,
            };
            let mut state = job.lock();
            state.finished_at = Some(Utc::now());
            match outcome {
                Some(Ok(output)) => {
                    let max = job.max_buffered_bytes;
                    state.stdout.push(output.as_bytes(), max);
                    state.status = BackgroundJobStatus::Exited;
                    state.success = true;
                }
                Some(Err(error)) => {
                    state.approval_required =
                        approval_required(&error).map(|required| required.reason().to_string());
                    state.status = BackgroundJobStatus::Failed;
                    state.error = Some(error.to_string());
                }
                None => {
                    state.status = BackgroundJobStatus::Cancelled;
                    state.error = Some("background task was cancelled".to_string());
                }
            }
            drop(state);
            finished.cancel();
        });
        Ok(snapshot)
    }

    /// Starts a persistent stdio session and returns once its process is running.
    /// Output is drained concurrently into the same bounded buffers used by
    /// detached commands, while stdin remains available through [`Self::write_stdin`].
    pub async fn spawn_session(
        &self,
        environment: Arc<dyn ExecutionEnvironment>,
        spawn: BackgroundSessionSpawnRequest,
    ) -> anyhow::Result<BackgroundJobSnapshot> {
        if !environment.supports_persistent_stdio() {
            anyhow::bail!(
                "execution environment '{}' does not support persistent stdio sessions",
                environment.id()
            );
        }

        let BackgroundSessionSpawnRequest {
            scope,
            command,
            request,
            mut context,
        } = spawn;
        let running = self
            .jobs()
            .values()
            .filter(|job| job.scope == scope && !job.lock().status.is_terminal())
            .count();
        if running >= self.inner.config.max_jobs_per_agent {
            anyhow::bail!(
                "this agent already has {running} background jobs running (maximum {}); wait for one to finish or stop it first",
                self.inner.config.max_jobs_per_agent
            );
        }

        let cancel = CancellationToken::new();
        let session_done = CancellationToken::new();
        let finished = CancellationToken::new();
        if let Some(turn_cancel) = context.cancel.take() {
            let session_cancel = cancel.clone();
            let session_finished = session_done.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = turn_cancel.cancelled() => session_cancel.cancel(),
                    _ = session_finished.cancelled() => {}
                }
            });
        }
        context.cancel = Some(cancel.clone());
        let timeout = context.timeout;
        let session: Arc<dyn StdioSession> = environment
            .spawn_stdio(request, context)
            .await
            .with_context(|| format!("failed to start interactive command: {command}"))?
            .into();

        let job = Arc::new(Job {
            id: Uuid::new_v4(),
            scope,
            command,
            started_at: Utc::now(),
            cancel: cancel.clone(),
            finished: finished.clone(),
            max_buffered_bytes: self.inner.config.max_buffered_bytes,
            session: Some(InteractiveSession {
                inner: session.clone(),
            }),
            state: Mutex::new(JobState {
                status: BackgroundJobStatus::Running,
                exit_code: None,
                success: false,
                finished_at: None,
                error: None,
                approval_required: None,
                truncated: false,
                sandbox: None,
                stdout: StreamBuffer::default(),
                stderr: StreamBuffer::default(),
                completion_reported: false,
            }),
        });
        let snapshot = job.snapshot();
        self.jobs().insert(job.id, job.clone());

        let stdout_job = job.clone();
        let stdout_session = session.clone();
        let stdout_task = tokio::spawn(async move {
            loop {
                match stdout_session.read_stdout().await {
                    Ok(bytes) if bytes.is_empty() => break,
                    Ok(bytes) => stdout_job
                        .lock()
                        .stdout
                        .push(&bytes, stdout_job.max_buffered_bytes),
                    Err(error) => {
                        let mut state = stdout_job.lock();
                        if state.error.is_none() {
                            state.error = Some(format!("failed to read session stdout: {error}"));
                        }
                        break;
                    }
                }
            }
        });
        let stderr_job = job.clone();
        let stderr_session = session.clone();
        let stderr_task = tokio::spawn(async move {
            loop {
                match stderr_session.read_stderr().await {
                    Ok(bytes) if bytes.is_empty() => break,
                    Ok(bytes) => stderr_job
                        .lock()
                        .stderr
                        .push(&bytes, stderr_job.max_buffered_bytes),
                    Err(error) => {
                        let mut state = stderr_job.lock();
                        if state.error.is_none() {
                            state.error = Some(format!("failed to read session stderr: {error}"));
                        }
                        break;
                    }
                }
            }
        });

        tokio::spawn(async move {
            // Do not cancel the `wait` future itself: LocalStdioSession owns the
            // child handle while waiting. Signal its cancellation token instead
            // so it can kill and reap the process without leaking it.
            let timed_out = Arc::new(AtomicBool::new(false));
            let timer = {
                let timed_out = timed_out.clone();
                let finished = session_done.clone();
                let cancel = cancel.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = tokio::time::sleep(timeout) => {
                            timed_out.store(true, Ordering::Release);
                            cancel.cancel();
                        }
                        _ = finished.cancelled() => {}
                    }
                })
            };
            let outcome = session.wait().await;
            session_done.cancel();
            let _ = timer.await;
            // Both pipes reach EOF after the process exits. Awaiting them before
            // publishing completion prevents the final log lines from arriving late.
            let _ = stdout_task.await;
            let _ = stderr_task.await;

            let mut state = job.lock();
            state.finished_at = Some(Utc::now());
            match outcome {
                Ok(result) => {
                    state.status = BackgroundJobStatus::Exited;
                    state.exit_code = result.exit_code;
                    state.success = result.success;
                    state.truncated = result.truncated;
                    state.sandbox = result.sandbox;
                }
                Err(error) => {
                    state.approval_required =
                        approval_required(&error).map(|required| required.reason().to_string());
                    let message = error.to_string();
                    state.status = if timed_out.load(Ordering::Acquire) {
                        BackgroundJobStatus::TimedOut
                    } else if job.cancel.is_cancelled() || message.contains("cancelled") {
                        BackgroundJobStatus::Cancelled
                    } else {
                        BackgroundJobStatus::Failed
                    };
                    if state.error.is_none() {
                        state.error = Some(message);
                    }
                }
            }
            drop(state);
            finished.cancel();
        });

        Ok(snapshot)
    }

    pub async fn write_stdin(
        &self,
        scope: &BackgroundScope,
        job_id: Uuid,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let job = self.visible(scope, job_id)?;
        if job.lock().status.is_terminal() {
            anyhow::bail!("background session {job_id} has already exited");
        }
        let session = job
            .session
            .as_ref()
            .context("background job is not an interactive stdio session")?
            .inner
            .clone();
        session.write_stdin(data).await
    }

    /// Returns the output produced since the agent last read, consuming it.
    pub fn read_output(
        &self,
        scope: &BackgroundScope,
        job_id: Uuid,
    ) -> anyhow::Result<BackgroundOutputChunk> {
        let job = self.visible(scope, job_id)?;
        let mut state = job.lock();
        let stdout = state.stdout.peek();
        let stderr = state.stderr.peek();
        let dropped = state.stdout.unread_dropped_bytes + state.stderr.unread_dropped_bytes;
        state.stdout.clear();
        state.stderr.clear();
        // Reading the tail of a finished job is the same delivery the push path would
        // have made, so it must not be announced twice.
        if state.status.is_terminal() {
            state.completion_reported = true;
        }
        drop(state);
        Ok(BackgroundOutputChunk {
            job: job.snapshot(),
            stdout,
            stderr,
            dropped_bytes: dropped,
        })
    }

    /// Waits briefly for a job to finish without turning a long command back into
    /// a blocking tool call. A completed job is consumed exactly like an explicit
    /// `background_output read`; `None` means it is still running.
    pub async fn wait_for_output(
        &self,
        scope: &BackgroundScope,
        job_id: Uuid,
        wait: Duration,
    ) -> anyhow::Result<Option<BackgroundOutputChunk>> {
        let job = self.visible(scope, job_id)?;
        if !job.lock().status.is_terminal() {
            let finished = job.finished.clone();
            // Register the waiter before checking again so a completion between
            // the first check and the select cannot be missed.
            let notified = finished.cancelled();
            if !job.lock().status.is_terminal()
                && tokio::time::timeout(wait, notified).await.is_err()
            {
                return Ok(None);
            }
        }
        self.read_output(scope, job_id).map(Some)
    }

    pub fn list(&self, scope: &BackgroundScope) -> Vec<BackgroundJobSnapshot> {
        let mut jobs = self
            .jobs()
            .values()
            .filter(|job| &job.scope == scope)
            .map(|job| job.snapshot())
            .collect::<Vec<_>>();
        jobs.sort_by_key(|job| job.started_at);
        jobs
    }

    pub fn stop(&self, scope: &BackgroundScope, job_id: Uuid) -> anyhow::Result<()> {
        self.visible(scope, job_id)?.cancel.cancel();
        Ok(())
    }

    /// Stops every job an agent owns. Used when a turn ends so no command outlives it
    /// unnoticed.
    pub fn stop_all(&self, scope: &BackgroundScope) -> usize {
        let mut stopped = 0;
        for job in self.jobs().values() {
            if &job.scope == scope && !job.lock().status.is_terminal() {
                job.cancel.cancel();
                stopped += 1;
            }
        }
        stopped
    }

    /// Jobs that finished since the agent last heard about them, without consuming
    /// anything. Confirm with [`Self::mark_reported`] once the model has actually seen
    /// them, so a failed round redelivers instead of losing the result.
    pub fn pending_completions(&self, scope: &BackgroundScope) -> Vec<BackgroundOutputChunk> {
        self.jobs()
            .values()
            .filter(|job| &job.scope == scope)
            .filter_map(|job| {
                let state = job.lock();
                if !state.status.is_terminal() || state.completion_reported {
                    return None;
                }
                let stdout = state.stdout.peek();
                let stderr = state.stderr.peek();
                let dropped = state.stdout.unread_dropped_bytes + state.stderr.unread_dropped_bytes;
                drop(state);
                Some(BackgroundOutputChunk {
                    job: job.snapshot(),
                    stdout,
                    stderr,
                    dropped_bytes: dropped,
                })
            })
            .collect()
    }

    pub fn mark_reported(&self, job_ids: &[Uuid]) {
        let jobs = self.jobs();
        for job_id in job_ids {
            if let Some(job) = jobs.get(job_id) {
                let mut state = job.lock();
                state.completion_reported = true;
                state.stdout.clear();
                state.stderr.clear();
            }
        }
    }

    /// Drops finished jobs whose result has already been delivered.
    pub fn prune_reported(&self, older_than: Duration) {
        let cutoff = Utc::now() - chrono::Duration::from_std(older_than).unwrap_or_default();
        self.jobs().retain(|_, job| {
            let state = job.lock();
            !(state.status.is_terminal()
                && state.completion_reported
                && state.finished_at.is_some_and(|at| at < cutoff))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::LocalExecutionEnvironment;
    use crate::sandbox::LocalSandboxConfig;

    fn workspace(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("opentopia-background-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create workspace");
        dir
    }

    fn environment(root: &std::path::Path) -> Arc<dyn ExecutionEnvironment> {
        Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            root.to_path_buf(),
            LocalSandboxConfig::danger_full_access(),
        ))
    }

    fn scope() -> BackgroundScope {
        BackgroundScope {
            thread_id: Uuid::new_v4(),
            agent_path: "/root".to_string(),
        }
    }

    async fn wait_until_terminal(
        registry: &BackgroundProcessRegistry,
        scope: &BackgroundScope,
        job_id: Uuid,
    ) -> BackgroundJobSnapshot {
        for _ in 0..200 {
            let snapshot = registry
                .list(scope)
                .into_iter()
                .find(|job| job.job_id == job_id)
                .expect("job is registered");
            if snapshot.status.is_terminal() {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("background job never reached a terminal state");
    }

    #[tokio::test]
    async fn a_background_command_returns_immediately_and_reports_when_it_finishes() {
        let root = workspace("completes");
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();

        let command = if cfg!(windows) {
            "Write-Output done"
        } else {
            "echo done"
        };
        let snapshot = registry
            .spawn(
                environment(&root),
                BackgroundSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(30)),
                },
            )
            .expect("spawn succeeds");

        // Spawning does not wait for the command.
        assert_eq!(snapshot.status, BackgroundJobStatus::Running);

        let finished = wait_until_terminal(&registry, &scope, snapshot.job_id).await;
        assert_eq!(finished.status, BackgroundJobStatus::Exited);

        // The completion is pending until the model is told about it.
        let pending = registry.pending_completions(&scope);
        assert_eq!(pending.len(), 1);
        assert!(pending[0].stdout.contains("done"));

        // A round that never reached the model redelivers.
        assert_eq!(registry.pending_completions(&scope).len(), 1);
        registry.mark_reported(&[snapshot.job_id]);
        assert!(registry.pending_completions(&scope).is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_short_wait_consumes_a_finished_job_without_polling() {
        let root = workspace("wait-finished");
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();
        let command = if cfg!(windows) {
            "Write-Output ready"
        } else {
            "echo ready"
        };
        let snapshot = registry
            .spawn(
                environment(&root),
                BackgroundSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(30)),
                },
            )
            .expect("spawn succeeds");

        let output = registry
            .wait_for_output(&scope, snapshot.job_id, Duration::from_secs(10))
            .await
            .expect("wait succeeds")
            .expect("quick command finishes inline");
        assert!(output.job.success);
        assert!(output.stdout.contains("ready"));
        assert!(registry.pending_completions(&scope).is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_short_wait_yields_a_job_that_is_still_running() {
        let root = workspace("wait-yields");
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();
        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let snapshot = registry
            .spawn(
                environment(&root),
                BackgroundSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(60)),
                },
            )
            .expect("spawn succeeds");

        let output = registry
            .wait_for_output(&scope, snapshot.job_id, Duration::from_millis(10))
            .await
            .expect("wait succeeds");
        assert!(
            output.is_none(),
            "a slow job must yield instead of blocking"
        );
        assert_eq!(
            registry.list(&scope)[0].status,
            BackgroundJobStatus::Running
        );

        registry
            .stop(&scope, snapshot.job_id)
            .expect("stop succeeds");
        wait_until_terminal(&registry, &scope, snapshot.job_id).await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_non_process_task_reuses_the_same_wait_and_completion_path() {
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();
        let snapshot = registry
            .spawn_task(scope.clone(), "download fixture".to_string(), None, async {
                Ok(r#"{"path":"fixture.zip","bytes":42}"#.to_string())
            })
            .expect("task starts");

        let output = registry
            .wait_for_output(&scope, snapshot.job_id, Duration::from_secs(1))
            .await
            .expect("wait succeeds")
            .expect("task finishes inline");
        assert!(output.job.success);
        assert!(output.stdout.contains("fixture.zip"));
        assert!(registry.pending_completions(&scope).is_empty());
    }

    #[tokio::test]
    async fn output_can_be_read_while_the_command_is_still_running() {
        let root = workspace("incremental");
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();

        // The window has to survive a loaded test machine, so the pause is generous.
        let command = if cfg!(windows) {
            "Write-Output first; Start-Sleep -Seconds 6; Write-Output second"
        } else {
            "echo first; sleep 6; echo second"
        };
        let snapshot = registry
            .spawn(
                environment(&root),
                BackgroundSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(30)),
                },
            )
            .expect("spawn succeeds");

        let mut seen_first_early = false;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let chunk = registry
                .read_output(&scope, snapshot.job_id)
                .expect("output is readable");
            if chunk.stdout.contains("first") && chunk.job.status == BackgroundJobStatus::Running {
                seen_first_early = true;
                break;
            }
        }
        assert!(
            seen_first_early,
            "early output should be readable before the command exits"
        );

        registry.stop(&scope, snapshot.job_id).ok();
        wait_until_terminal(&registry, &scope, snapshot.job_id).await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stopping_a_job_ends_the_command() {
        let root = workspace("stop");
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();

        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 120"
        } else {
            "sleep 120"
        };
        let snapshot = registry
            .spawn(
                environment(&root),
                BackgroundSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(300)),
                },
            )
            .expect("spawn succeeds");

        registry
            .stop(&scope, snapshot.job_id)
            .expect("stop succeeds");
        let finished = wait_until_terminal(&registry, &scope, snapshot.job_id).await;
        assert!(finished.status.is_terminal());

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn another_agent_cannot_read_or_stop_the_job() {
        let root = workspace("scoped");
        let registry = BackgroundProcessRegistry::default();
        let owner = scope();
        let command = if cfg!(windows) {
            "Write-Output mine"
        } else {
            "echo mine"
        };
        let snapshot = registry
            .spawn(
                environment(&root),
                BackgroundSpawnRequest {
                    scope: owner.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(30)),
                },
            )
            .expect("spawn succeeds");

        let stranger = BackgroundScope {
            thread_id: owner.thread_id,
            agent_path: "/root/other".to_string(),
        };
        assert!(registry.read_output(&stranger, snapshot.job_id).is_err());
        assert!(registry.stop(&stranger, snapshot.job_id).is_err());
        assert!(registry.list(&stranger).is_empty());

        wait_until_terminal(&registry, &owner, snapshot.job_id).await;
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_stream_keeps_the_first_and_last_bytes() {
        let mut buffer = StreamBuffer::default();
        buffer.push(b"BEGIN-0123456789-END", 12);
        let visible = buffer.peek();
        assert!(visible.starts_with("BEGIN-"));
        assert!(visible.ends_with("89-END"));
        assert_eq!(buffer.unread.len(), 12);
        assert_eq!(buffer.unread_dropped_bytes, 8);

        buffer.clear();
        assert_eq!(buffer.unread_dropped_bytes, 0);
        assert_eq!(buffer.dropped_bytes, 8);
    }

    #[tokio::test]
    async fn persistent_session_accepts_input_and_reports_output() {
        let root = workspace("interactive");
        let registry = BackgroundProcessRegistry::default();
        let scope = scope();
        let command = if cfg!(windows) {
            "$line = [Console]::In.ReadLine(); Write-Output \"got:$line\""
        } else {
            "read line; echo got:$line"
        };
        let snapshot = registry
            .spawn_session(
                environment(&root),
                BackgroundSessionSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: ExecRequest::shell(command).cwd(&root),
                    context: ExecutionContext::with_timeout(Duration::from_secs(30)),
                },
            )
            .await
            .expect("session starts");
        assert!(snapshot.interactive);

        registry
            .write_stdin(&scope, snapshot.job_id, b"hello\n")
            .await
            .expect("stdin write succeeds");
        let finished = wait_until_terminal(&registry, &scope, snapshot.job_id).await;
        assert_eq!(finished.status, BackgroundJobStatus::Exited);
        let output = registry
            .read_output(&scope, snapshot.job_id)
            .expect("session output is readable");
        assert!(output.stdout.contains("got:hello"), "{:?}", output);

        let _ = std::fs::remove_dir_all(root);
    }
}
