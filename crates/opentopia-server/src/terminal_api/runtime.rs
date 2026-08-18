use super::{
    ApiError, AppState, TerminalCancelResponse, TerminalSessionResponse, SENSITIVE_CHILD_ENV_KEYS,
    TERMINAL_HISTORY_LIMIT, TERMINAL_OUTPUT_BYTES_LIMIT,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use opentopia_core::{
    current_shell_runtime, SessionStore, SqliteSessionStore, TerminalCommandHistory,
    TerminalCommandStatus, TerminalEvent, TerminalEventKind,
};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::{broadcast, oneshot};
use tokio::time::timeout;
use tracing::error;
use uuid::Uuid;

#[derive(Clone, Default)]
pub(crate) struct TerminalBus {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<TerminalEvent>>>>,
    histories: Arc<RwLock<HashMap<Uuid, Vec<TerminalEvent>>>>,
    next_seq: Arc<RwLock<HashMap<Uuid, u64>>>,
    running: Arc<RwLock<HashMap<Uuid, RunningTerminalCommand>>>,
}

struct RunningTerminalCommand {
    command_id: Uuid,
    cancel: oneshot::Sender<()>,
}

impl TerminalBus {
    pub(super) fn subscribe(&self, thread_id: Uuid) -> broadcast::Receiver<TerminalEvent> {
        let mut channels = self.channels.write().expect("terminal bus poisoned");
        channels
            .entry(thread_id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(512);
                tx
            })
            .subscribe()
    }

    pub(super) fn history(&self, thread_id: Uuid, since: Option<u64>) -> Vec<TerminalEvent> {
        let histories = self.histories.read().expect("terminal history poisoned");
        histories
            .get(&thread_id)
            .into_iter()
            .flatten()
            .filter(|event| since.map_or(true, |seq| event.seq > seq))
            .cloned()
            .collect()
    }

    pub(super) fn ensure_min_seq(&self, thread_id: Uuid, min_seq: u64) {
        let mut next_seq = self.next_seq.write().expect("terminal seq poisoned");
        let entry = next_seq.entry(thread_id).or_insert(0);
        if *entry < min_seq {
            *entry = min_seq;
        }
    }

    pub(super) fn register_running(
        &self,
        thread_id: Uuid,
        command_id: Uuid,
        cancel: oneshot::Sender<()>,
    ) -> Result<(), ApiError> {
        let mut running = self.running.write().expect("terminal running poisoned");
        if let Some(existing) = running.get(&thread_id) {
            return Err(ApiError::conflict(format!(
                "terminal command already running: {}",
                existing.command_id
            )));
        }
        running.insert(thread_id, RunningTerminalCommand { command_id, cancel });
        Ok(())
    }

    pub(super) fn cancel_running(
        &self,
        thread_id: Uuid,
        requested_command_id: Option<Uuid>,
    ) -> TerminalCancelResponse {
        let mut running = self.running.write().expect("terminal running poisoned");
        let Some(active) = running.get(&thread_id) else {
            return TerminalCancelResponse {
                command_id: requested_command_id,
                cancelled: false,
                message: "no running terminal command".to_string(),
            };
        };

        if let Some(command_id) = requested_command_id {
            if active.command_id != command_id {
                return TerminalCancelResponse {
                    command_id: Some(command_id),
                    cancelled: false,
                    message: format!(
                        "running terminal command is {}, not {}",
                        active.command_id, command_id
                    ),
                };
            }
        }

        let active = running
            .remove(&thread_id)
            .expect("running command disappeared");
        let command_id = active.command_id;
        let _ = active.cancel.send(());
        TerminalCancelResponse {
            command_id: Some(command_id),
            cancelled: true,
            message: "cancel requested".to_string(),
        }
    }

    pub(super) fn remove_running(&self, thread_id: Uuid, command_id: Uuid) {
        let mut running = self.running.write().expect("terminal running poisoned");
        if running
            .get(&thread_id)
            .is_some_and(|active| active.command_id == command_id)
        {
            running.remove(&thread_id);
        }
    }

    pub(super) fn publish_event(
        &self,
        thread_id: Uuid,
        command_id: Uuid,
        kind: TerminalEventKind,
        fields: TerminalEventFields,
    ) -> TerminalEvent {
        let seq = {
            let mut next_seq = self.next_seq.write().expect("terminal seq poisoned");
            let entry = next_seq.entry(thread_id).or_insert(0);
            *entry += 1;
            *entry
        };
        let event = TerminalEvent {
            id: Uuid::new_v4(),
            thread_id,
            command_id,
            seq,
            created_at: Utc::now(),
            kind,
            command: fields.command,
            cwd: fields.cwd,
            data: fields.data,
            exit_code: fields.exit_code,
            success: fields.success,
            message: fields.message,
        };

        {
            let mut histories = self.histories.write().expect("terminal history poisoned");
            let history = histories.entry(thread_id).or_default();
            history.push(event.clone());
            if history.len() > TERMINAL_HISTORY_LIMIT {
                let overflow = history.len() - TERMINAL_HISTORY_LIMIT;
                history.drain(0..overflow);
            }
        }

        let sender = {
            let mut channels = self.channels.write().expect("terminal bus poisoned");
            channels
                .entry(thread_id)
                .or_insert_with(|| {
                    let (tx, _rx) = broadcast::channel(512);
                    tx
                })
                .clone()
        };
        let _ = sender.send(event.clone());
        event
    }
}

const PTY_OUTPUT_HISTORY_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct PtyManager {
    sessions: Arc<RwLock<HashMap<Uuid, Arc<PtySession>>>>,
}

pub(super) struct PtySession {
    pub(super) session_id: Uuid,
    thread_id: Uuid,
    cwd: PathBuf,
    shell: String,
    process_id: Option<u32>,
    started_at: DateTime<Utc>,
    seq_start: u64,
    running: AtomicBool,
    close_requested: AtomicBool,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<String>,
}

impl PtyManager {
    pub(super) fn get(&self, thread_id: Uuid) -> Option<Arc<PtySession>> {
        self.sessions
            .read()
            .expect("pty sessions poisoned")
            .get(&thread_id)
            .filter(|session| session.running.load(Ordering::SeqCst))
            .cloned()
    }

    pub(super) fn insert(&self, session: Arc<PtySession>) {
        self.sessions
            .write()
            .expect("pty sessions poisoned")
            .insert(session.thread_id, session);
    }

    fn remove_if(&self, thread_id: Uuid, session_id: Uuid) {
        let mut sessions = self.sessions.write().expect("pty sessions poisoned");
        if sessions
            .get(&thread_id)
            .is_some_and(|session| session.session_id == session_id)
        {
            sessions.remove(&thread_id);
        }
    }
}

impl PtySession {
    pub(super) fn view(&self) -> TerminalSessionResponse {
        TerminalSessionResponse {
            session_id: self.session_id,
            thread_id: self.thread_id,
            status: if self.running.load(Ordering::SeqCst) {
                "running"
            } else {
                "closed"
            },
            cwd: shell_native_path(&self.cwd),
            shell: self.shell.clone(),
            process_id: self.process_id,
            started_at: self.started_at,
        }
    }

    pub(super) fn write(&self, data: &str) -> anyhow::Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            anyhow::bail!("terminal session is closed");
        }
        let mut writer = self.writer.lock().expect("pty writer poisoned");
        let writer = writer
            .as_mut()
            .context("terminal session input is closed")?;
        writer.write_all(data.as_bytes())?;
        writer.flush()?;
        Ok(())
    }

    pub(super) fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        if cols == 0 || rows == 0 {
            anyhow::bail!("terminal size must be greater than zero");
        }
        let master = self.master.lock().expect("pty master poisoned");
        master
            .as_ref()
            .context("terminal session is closed")?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
        Ok(())
    }

    pub(super) fn kill(&self) -> anyhow::Result<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.close_requested.store(true, Ordering::SeqCst);
        self.writer.lock().expect("pty writer poisoned").take();
        self.master.lock().expect("pty master poisoned").take();
        #[cfg(windows)]
        if let Some(process_id) = self.process_id {
            use std::os::windows::process::CommandExt;

            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let status = std::process::Command::new("taskkill")
                .args(["/PID", &process_id.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status();
            if status.is_ok_and(|status| status.success()) {
                return Ok(());
            }
        }
        match self.killer.lock().expect("pty killer poisoned").kill() {
            Ok(()) => Ok(()),
            // portable-pty 0.9's WinChildKiller inverts the TerminateProcess
            // return check. A successful termination is surfaced as os error 0.
            #[cfg(windows)]
            Err(err) if err.raw_os_error() == Some(0) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    fn append_output(&self, chunk: &str) {
        let mut output = self.output.lock().expect("pty output poisoned");
        output.push_str(chunk);
        if output.len() > PTY_OUTPUT_HISTORY_LIMIT {
            let mut start = output.len() - PTY_OUTPUT_HISTORY_LIMIT;
            while !output.is_char_boundary(start) {
                start += 1;
            }
            output.drain(..start);
        }
    }
}

pub(super) fn spawn_pty_session(
    state: AppState,
    thread_id: Uuid,
    cwd: PathBuf,
    cols: u16,
    rows: u16,
) -> Result<Arc<PtySession>, ApiError> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let (shell, shell_args) = interactive_shell();
    // This PTY is the user's integrated terminal, equivalent to opening
    // PowerShell in VS Code. Agent-initiated commands continue to use the
    // sandboxed execution routes; direct terminal input runs as the user.
    let mut command = CommandBuilder::new(&shell);
    command.cwd(shell_native_path(&cwd));
    for key in SENSITIVE_CHILD_ENV_KEYS {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for arg in &shell_args {
        command.arg(arg);
    }

    let mut child = pair.slave.spawn_command(command)?;
    let process_id = child.process_id();
    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let session_id = Uuid::new_v4();
    let cwd_display = shell_native_path(&cwd).to_string_lossy().to_string();
    let started_event = state.terminals.publish_event(
        thread_id,
        session_id,
        TerminalEventKind::Started,
        TerminalEventFields {
            command: Some(format!("interactive {shell}")),
            cwd: Some(cwd_display),
            message: Some("persistent PTY session started".to_string()),
            ..Default::default()
        },
    );
    let session = Arc::new(PtySession {
        session_id,
        thread_id,
        cwd: cwd.clone(),
        shell: shell.clone(),
        process_id,
        started_at: started_event.created_at,
        seq_start: started_event.seq,
        running: AtomicBool::new(true),
        close_requested: AtomicBool::new(false),
        writer: Mutex::new(Some(writer)),
        master: Mutex::new(Some(pair.master)),
        killer: Mutex::new(killer),
        output: Mutex::new(String::new()),
    });

    let reader_session = session.clone();
    let reader_terminals = state.terminals.clone();
    let reader_handle = std::thread::Builder::new()
        .name(format!("opentopia-pty-reader-{session_id}"))
        .spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        let chunk = String::from_utf8_lossy(&buffer[..size]).to_string();
                        reader_session.append_output(&chunk);
                        reader_terminals.publish_event(
                            thread_id,
                            session_id,
                            TerminalEventKind::Stdout,
                            TerminalEventFields {
                                data: Some(chunk),
                                ..Default::default()
                            },
                        );
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(err) => {
                        if reader_session.running.load(Ordering::SeqCst) {
                            reader_terminals.publish_event(
                                thread_id,
                                session_id,
                                TerminalEventKind::Error,
                                TerminalEventFields {
                                    success: Some(false),
                                    message: Some(format!("PTY read failed: {err}")),
                                    ..Default::default()
                                },
                            );
                        }
                        break;
                    }
                }
            }
        })?;

    let supervisor_session = session.clone();
    let supervisor_state = state.clone();
    std::thread::Builder::new()
        .name(format!("opentopia-pty-supervisor-{session_id}"))
        .spawn(move || {
            let status = child.wait();
            supervisor_session.running.store(false, Ordering::SeqCst);
            let _ = reader_handle.join();
            let close_requested = supervisor_session.close_requested.load(Ordering::SeqCst);
            let (kind, command_status, exit_code, success, message) = match status {
                Ok(status) if close_requested => (
                    TerminalEventKind::Cancelled,
                    TerminalCommandStatus::Cancelled,
                    Some(status.exit_code() as i32),
                    false,
                    Some("persistent PTY session closed".to_string()),
                ),
                Ok(status) => {
                    let code = status.exit_code() as i32;
                    let ok = code == 0;
                    (
                        TerminalEventKind::Finished,
                        if ok {
                            TerminalCommandStatus::Finished
                        } else {
                            TerminalCommandStatus::Failed
                        },
                        Some(code),
                        ok,
                        (!ok).then(|| format!("PTY shell exited with code {code}")),
                    )
                }
                Err(err) => (
                    TerminalEventKind::Error,
                    TerminalCommandStatus::Error,
                    None,
                    false,
                    Some(format!("PTY wait failed: {err}")),
                ),
            };
            let final_event = supervisor_state.terminals.publish_event(
                thread_id,
                session_id,
                kind,
                TerminalEventFields {
                    exit_code,
                    success: Some(success),
                    message: message.clone(),
                    ..Default::default()
                },
            );
            let output = supervisor_session
                .output
                .lock()
                .expect("pty output poisoned")
                .clone();
            if let Err(err) =
                supervisor_state
                    .store
                    .insert_terminal_history(TerminalCommandHistory {
                        command_id: session_id,
                        thread_id,
                        seq_start: supervisor_session.seq_start,
                        seq_end: final_event.seq,
                        command: format!("interactive {}", supervisor_session.shell),
                        cwd: Some(supervisor_session.cwd.clone()),
                        stdout: output,
                        stderr: String::new(),
                        exit_code,
                        status: command_status,
                        message,
                        started_at: supervisor_session.started_at,
                        completed_at: final_event.created_at,
                    })
            {
                error!(?err, %thread_id, %session_id, "failed to persist PTY history");
            }
            supervisor_state.ptys.remove_if(thread_id, session_id);
        })?;

    Ok(session)
}

fn interactive_shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        let runtime = current_shell_runtime();
        (
            runtime.program.to_string_lossy().into_owned(),
            vec!["-NoProfile".to_string()],
        )
    } else {
        (
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            vec!["-l".to_string()],
        )
    }
}

fn shell_native_path(path: &FsPath) -> PathBuf {
    #[cfg(windows)]
    {
        let display = path.as_os_str().to_string_lossy();
        if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(native) = display.strip_prefix(r"\\?\") {
            return PathBuf::from(native);
        }
    }
    path.to_path_buf()
}

pub(super) async fn run_terminal_command(
    mut child: tokio::process::Child,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    mut cancel_rx: oneshot::Receiver<()>,
    terminals: TerminalBus,
    store: Arc<SqliteSessionStore>,
    thread_id: Uuid,
    command_id: Uuid,
    command: String,
    cwd: PathBuf,
    seq_start: u64,
    started_at: DateTime<Utc>,
    timeout_ms: u64,
) {
    let child_pid = child.id();
    let stdout_task = stdout.map(|pipe| {
        tokio::spawn(read_terminal_pipe(
            pipe,
            TerminalEventKind::Stdout,
            terminals.clone(),
            thread_id,
            command_id,
        ))
    });
    let stderr_task = stderr.map(|pipe| {
        tokio::spawn(read_terminal_pipe(
            pipe,
            TerminalEventKind::Stderr,
            terminals.clone(),
            thread_id,
            command_id,
        ))
    });

    let timeout_sleep = tokio::time::sleep(Duration::from_millis(timeout_ms));
    tokio::pin!(timeout_sleep);

    enum TerminalCompletion {
        Exited(std::io::Result<std::process::ExitStatus>),
        Cancelled,
        TimedOut,
    }

    let completion = tokio::select! {
        result = child.wait() => TerminalCompletion::Exited(result),
        _ = &mut cancel_rx => TerminalCompletion::Cancelled,
        _ = &mut timeout_sleep => TerminalCompletion::TimedOut,
    };

    let (final_kind, final_event, history_status) = match completion {
        TerminalCompletion::Exited(Ok(status)) => {
            let success = status.success();
            (
                TerminalEventKind::Finished,
                TerminalEventFields {
                    exit_code: status.code(),
                    success: Some(success),
                    message: (!success).then(|| {
                        status
                            .code()
                            .map(|code| format!("command exited with code {code}"))
                            .unwrap_or_else(|| "command terminated by signal".to_string())
                    }),
                    ..Default::default()
                },
                if success {
                    TerminalCommandStatus::Finished
                } else {
                    TerminalCommandStatus::Failed
                },
            )
        }
        TerminalCompletion::Exited(Err(err)) => (
            TerminalEventKind::Error,
            TerminalEventFields {
                success: Some(false),
                message: Some(err.to_string()),
                ..Default::default()
            },
            TerminalCommandStatus::Error,
        ),
        TerminalCompletion::Cancelled => {
            let cleanup_message = terminate_terminal_child(&mut child, child_pid).await;
            (
                TerminalEventKind::Cancelled,
                TerminalEventFields {
                    success: Some(false),
                    message: Some(format!("command cancelled; {cleanup_message}")),
                    ..Default::default()
                },
                TerminalCommandStatus::Cancelled,
            )
        }
        TerminalCompletion::TimedOut => {
            let cleanup_message = terminate_terminal_child(&mut child, child_pid).await;
            (
                TerminalEventKind::Error,
                TerminalEventFields {
                    success: Some(false),
                    message: Some(format!(
                        "command timed out after {timeout_ms}ms; {cleanup_message}"
                    )),
                    ..Default::default()
                },
                TerminalCommandStatus::TimedOut,
            )
        }
    };

    let stdout = match stdout_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    let stderr = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };

    terminals.remove_running(thread_id, command_id);
    let terminal_event = terminals.publish_event(thread_id, command_id, final_kind, final_event);
    let history = TerminalCommandHistory {
        command_id,
        thread_id,
        seq_start,
        seq_end: terminal_event.seq,
        command,
        cwd: Some(cwd),
        stdout,
        stderr,
        exit_code: terminal_event.exit_code,
        status: history_status,
        message: terminal_event.message.clone(),
        started_at,
        completed_at: terminal_event.created_at,
    };
    if let Err(err) = store.insert_terminal_history(history) {
        error!(?err, %thread_id, %command_id, "failed to persist terminal history");
    }
}

async fn terminate_terminal_child(
    child: &mut tokio::process::Child,
    child_pid: Option<u32>,
) -> String {
    match child.try_wait() {
        Ok(Some(status)) => return format!("process already exited with {status}"),
        Ok(None) => {}
        Err(err) => return format!("could not inspect child process: {err}"),
    }

    #[cfg(windows)]
    let request = if let Some(pid) = child_pid {
        match Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .await
        {
            Ok(output) if output.status.success() => {
                "process tree termination requested".to_string()
            }
            Ok(output) => {
                let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let fallback = child.start_kill();
                format!(
                    "taskkill failed{}; direct termination {}",
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    },
                    if fallback.is_ok() {
                        "requested"
                    } else {
                        "failed"
                    }
                )
            }
            Err(err) => {
                let fallback = child.start_kill();
                format!(
                    "taskkill could not start ({err}); direct termination {}",
                    if fallback.is_ok() {
                        "requested"
                    } else {
                        "failed"
                    }
                )
            }
        }
    } else {
        let result = child.start_kill();
        format!(
            "direct termination {}",
            if result.is_ok() {
                "requested"
            } else {
                "failed"
            }
        )
    };

    #[cfg(not(windows))]
    let request = {
        let result = child.start_kill();
        format!(
            "process termination {}",
            if result.is_ok() {
                "requested"
            } else {
                "failed"
            }
        )
    };

    match timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => format!("{request}; process exited with {status}"),
        Ok(Err(err)) => format!("{request}; failed to reap process: {err}"),
        Err(_) => format!("{request}; process did not exit within 5 seconds"),
    }
}

async fn read_terminal_pipe<R>(
    mut reader: R,
    kind: TerminalEventKind,
    terminals: TerminalBus,
    thread_id: Uuid,
    command_id: Uuid,
) -> String
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0u8; 8192];
    let mut output = String::new();
    let mut truncation_reported = false;
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(n) => {
                if output.len() < TERMINAL_OUTPUT_BYTES_LIMIT {
                    let remaining = TERMINAL_OUTPUT_BYTES_LIMIT - output.len();
                    let accepted = n.min(remaining);
                    let chunk = String::from_utf8_lossy(&buffer[..accepted]).to_string();
                    output.push_str(&chunk);
                    terminals.publish_event(
                        thread_id,
                        command_id,
                        kind,
                        TerminalEventFields {
                            data: Some(chunk),
                            ..Default::default()
                        },
                    );
                    if accepted < n && !truncation_reported {
                        truncation_reported = true;
                        let marker = "\n[terminal output truncated at 4 MiB]\n";
                        output.push_str(marker);
                        terminals.publish_event(
                            thread_id,
                            command_id,
                            kind,
                            TerminalEventFields {
                                data: Some(marker.to_string()),
                                ..Default::default()
                            },
                        );
                    }
                } else if !truncation_reported {
                    truncation_reported = true;
                    let marker = "\n[terminal output truncated at 4 MiB]\n";
                    output.push_str(marker);
                    terminals.publish_event(
                        thread_id,
                        command_id,
                        kind,
                        TerminalEventFields {
                            data: Some(marker.to_string()),
                            ..Default::default()
                        },
                    );
                }
            }
            Err(err) => {
                let stream = if kind == TerminalEventKind::Stdout {
                    "stdout"
                } else {
                    "stderr"
                };
                terminals.publish_event(
                    thread_id,
                    command_id,
                    TerminalEventKind::Error,
                    TerminalEventFields {
                        success: Some(false),
                        message: Some(format!("failed to read terminal {stream}: {err}")),
                        ..Default::default()
                    },
                );
                break;
            }
        }
    }
    output
}

#[derive(Debug, Default)]
pub(super) struct TerminalEventFields {
    pub(super) command: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) data: Option<String>,
    pub(super) exit_code: Option<i32>,
    pub(super) success: Option<bool>,
    pub(super) message: Option<String>,
}
