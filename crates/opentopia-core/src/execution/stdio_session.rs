//! Persistent stdio session lifecycle and cancellation semantics.

use super::local_environment::LocalExecutionEnvironment;
use super::{
    sandbox_infrastructure_error, ExecResult, ExecutionSandboxMetadata, StdioSession,
    SANDBOX_ERROR_EXIT_CODE,
};
use crate::execution_spec::{ExecutionFailure, ExecutionStage};
use crate::process_quota::ProcessQuota;
use crate::process_supervisor::terminate_process;
use async_trait::async_trait;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

pub struct LocalStdioSession {
    pub(super) child: tokio::sync::Mutex<Option<tokio::process::Child>>,
    pub(super) stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pub(super) stdout: tokio::sync::Mutex<tokio::process::ChildStdout>,
    pub(super) stderr: tokio::sync::Mutex<tokio::process::ChildStderr>,
    pub(super) cancel: Option<CancellationToken>,
    pub(super) cancel_token: Option<CancellationToken>,
    pub(super) request_id: Option<String>,
    pub(super) env: Option<std::sync::Arc<LocalExecutionEnvironment>>,
    pub(super) sandbox: Option<ExecutionSandboxMetadata>,
    pub(super) sandbox_error_nonce: Option<String>,
    pub(super) stderr_observed: tokio::sync::Mutex<Vec<u8>>,
    pub(super) execution_timeout: Duration,
    pub(super) termination_timeout: Duration,
    /// Kept for the session's lifetime. The job terminates its members when the last handle
    /// closes, so releasing this before the session ends would kill the child.
    pub(super) quota: tokio::sync::Mutex<Option<ProcessQuota>>,
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
