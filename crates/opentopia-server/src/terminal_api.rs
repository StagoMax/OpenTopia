use super::{ensure_thread, ApiError, AppState};
use crate::workspace_api::canonical_workspace_root;
mod runtime;

use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use opentopia_core::{
    current_shell_runtime, DesktopStreamEnvelope, DesktopStreamKind, SessionStore,
    TerminalCommandHistory, TerminalCommandStatus, TerminalEvent, TerminalEventKind,
};
use runtime::{run_terminal_command, spawn_pty_session, PtySession, TerminalEventFields};
pub(super) use runtime::{PtyManager, TerminalBus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

const TERMINAL_HISTORY_LIMIT: usize = 2_000;
const DEFAULT_TERMINAL_TIMEOUT_MS: u64 = 300_000;
const TERMINAL_OUTPUT_BYTES_LIMIT: usize = 4 * 1024 * 1024;
const SENSITIVE_CHILD_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENTOPIA_API_KEY",
    "OPENTOPIA_API_TOKEN",
    "CREDIT_REVIEW_LLM_API_KEY",
];
const MAX_TERMINAL_TIMEOUT_MS: u64 = 3_600_000;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/threads/:thread_id/terminal/commands",
            post(start_terminal_command),
        )
        .route(
            "/api/threads/:thread_id/terminal/cancel",
            post(cancel_terminal_command),
        )
        .route(
            "/api/threads/:thread_id/terminal/history",
            get(list_terminal_history),
        )
        .route(
            "/api/threads/:thread_id/terminal/stream",
            get(stream_terminal_events),
        )
        .route(
            "/api/threads/:thread_id/terminal/session",
            get(get_terminal_session).post(ensure_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/terminal/session/input",
            post(write_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/terminal/session/resize",
            post(resize_terminal_session),
        )
        .route(
            "/api/threads/:thread_id/terminal/session/close",
            post(close_terminal_session),
        )
}

async fn get_terminal_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<TerminalSessionResponse>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(
        state.ptys.get(thread_id).map(|session| session.view()),
    ))
}

async fn ensure_terminal_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    request: Option<Json<TerminalSessionCreateRequest>>,
) -> Result<Json<TerminalSessionResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    if let Some(session) = state.ptys.get(thread_id) {
        return Ok(Json(session.view()));
    }

    state.terminals.ensure_min_seq(
        thread_id,
        state.store.latest_terminal_history_seq(thread_id)?,
    );
    let request = request.map(|Json(value)| value).unwrap_or_default();
    let cols = request.cols.unwrap_or(100).clamp(20, 500);
    let rows = request.rows.unwrap_or(30).clamp(5, 200);
    let cwd = resolve_terminal_cwd(&thread.workspace_root, request.cwd.as_deref())?;
    let session = spawn_pty_session(state.clone(), thread_id, cwd, cols, rows)?;
    state.ptys.insert(session.clone());
    Ok(Json(session.view()))
}

async fn write_terminal_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<TerminalSessionInputRequest>,
) -> Result<Json<TerminalSessionResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    if request.data.len() > 64 * 1024 {
        return Err(ApiError::bad_request("terminal input exceeds 64 KiB"));
    }
    let session = require_pty_session(&state, thread_id, request.session_id)?;
    session.write(&request.data)?;
    Ok(Json(session.view()))
}

async fn resize_terminal_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<TerminalSessionResizeRequest>,
) -> Result<Json<TerminalSessionResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = require_pty_session(&state, thread_id, request.session_id)?;
    session.resize(request.cols.clamp(20, 500), request.rows.clamp(5, 200))?;
    Ok(Json(session.view()))
}

async fn close_terminal_session(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<TerminalSessionCloseRequest>,
) -> Result<Json<TerminalSessionResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let session = require_pty_session(&state, thread_id, request.session_id)?;
    session.kill()?;
    Ok(Json(session.view()))
}

fn require_pty_session(
    state: &AppState,
    thread_id: Uuid,
    session_id: Uuid,
) -> Result<Arc<PtySession>, ApiError> {
    let session = state
        .ptys
        .get(thread_id)
        .ok_or_else(|| ApiError::not_found("terminal session not found"))?;
    if session.session_id != session_id {
        return Err(ApiError::conflict(format!(
            "active terminal session is {}, not {}",
            session.session_id, session_id
        )));
    }
    Ok(session)
}

async fn start_terminal_command(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<TerminalStartRequest>,
) -> Result<Json<TerminalStartResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let command = request.command.trim().to_string();
    if command.is_empty() {
        return Err(ApiError::bad_request("terminal command cannot be empty"));
    }

    let cwd = resolve_terminal_cwd(&thread.workspace_root, request.cwd.as_deref())?;
    let timeout_ms = request
        .timeout_ms
        .unwrap_or(DEFAULT_TERMINAL_TIMEOUT_MS)
        .clamp(1_000, MAX_TERMINAL_TIMEOUT_MS);
    state.terminals.ensure_min_seq(
        thread_id,
        state.store.latest_terminal_history_seq(thread_id)?,
    );
    let command_id = Uuid::new_v4();
    let (cancel_tx, cancel_rx) = oneshot::channel();
    state
        .terminals
        .register_running(thread_id, command_id, cancel_tx)?;

    // This terminal is driven directly by the signed-in desktop user, just like
    // the persistent PTY below. Agent shell calls still go through the execution
    // environment and its sandbox; wrapping the user's terminal here only adds
    // ACL setup latency and can serialize it behind unrelated agent work.
    let (program, args) = if cfg!(windows) {
        let runtime = current_shell_runtime();
        (
            runtime.program.to_string_lossy().into_owned(),
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                command.clone(),
            ],
        )
    } else {
        ("sh".to_string(), vec!["-lc".to_string(), command.clone()])
    };
    let mut process = Command::new(program);
    for key in SENSITIVE_CHILD_ENV_KEYS {
        process.env_remove(key);
    }
    process
        .args(args)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(err) => {
            state.terminals.remove_running(thread_id, command_id);
            let message = err.to_string();
            let error_event = state.terminals.publish_event(
                thread_id,
                command_id,
                TerminalEventKind::Error,
                TerminalEventFields {
                    command: Some(command.clone()),
                    cwd: Some(cwd.to_string_lossy().to_string()),
                    message: Some(message.clone()),
                    success: Some(false),
                    ..Default::default()
                },
            );
            state
                .store
                .insert_terminal_history(TerminalCommandHistory {
                    command_id,
                    thread_id,
                    seq_start: error_event.seq,
                    seq_end: error_event.seq,
                    command,
                    cwd: Some(cwd),
                    stdout: String::new(),
                    stderr: message.clone(),
                    exit_code: None,
                    status: TerminalCommandStatus::Error,
                    message: Some(message),
                    started_at: error_event.created_at,
                    completed_at: error_event.created_at,
                })?;
            return Err(ApiError::from(anyhow::Error::from(err)));
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let terminals = state.terminals.clone();
    let store = state.store.clone();
    let cwd_display = cwd.to_string_lossy().to_string();
    let started_event = terminals.publish_event(
        thread_id,
        command_id,
        TerminalEventKind::Started,
        TerminalEventFields {
            command: Some(command.clone()),
            cwd: Some(cwd_display.clone()),
            ..Default::default()
        },
    );

    tokio::spawn(run_terminal_command(
        child,
        stdout,
        stderr,
        cancel_rx,
        terminals,
        store,
        thread_id,
        command_id,
        command,
        cwd,
        started_event.seq,
        started_event.created_at,
        timeout_ms,
    ));

    Ok(Json(TerminalStartResponse {
        thread_id,
        command_id,
        status: "started",
        history_url: format!("/api/threads/{thread_id}/terminal/history"),
        stream_url: format!("/api/threads/{thread_id}/terminal/stream"),
    }))
}

async fn cancel_terminal_command(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<TerminalCancelRequest>,
) -> Result<Json<TerminalCancelResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(
        state
            .terminals
            .cancel_running(thread_id, request.command_id),
    ))
}

async fn list_terminal_history(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<TerminalQuery>,
) -> Result<Json<Vec<TerminalEvent>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let persisted_max_seq = state.store.latest_terminal_history_seq(thread_id)?;
    let mut history = terminal_events_from_persistent_history(&state, thread_id, query.since)?;
    history.extend(
        state
            .terminals
            .history(thread_id, query.since)
            .into_iter()
            .filter(|event| event.seq > persisted_max_seq),
    );
    history.sort_by_key(|event| event.seq);
    Ok(Json(history))
}

async fn stream_terminal_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<TerminalQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let rx = state.terminals.subscribe(thread_id);
    let persisted_max_seq = state.store.latest_terminal_history_seq(thread_id)?;
    let mut history = terminal_events_from_persistent_history(&state, thread_id, query.since)?;
    history.extend(
        state
            .terminals
            .history(thread_id, query.since)
            .into_iter()
            .filter(|event| event.seq > persisted_max_seq),
    );
    history.sort_by_key(|event| event.seq);
    let history_stream = stream::iter(history);
    let live_stream = BroadcastStream::new(rx).filter_map(|event| async move { event.ok() });
    let event_stream = history_stream.chain(live_stream).map(|terminal_event| {
        let seq = i64::try_from(terminal_event.seq).expect("terminal sequence should fit in i64");
        let envelope =
            DesktopStreamEnvelope::new(DesktopStreamKind::TerminalEvent, seq, terminal_event);
        let sse = Event::default()
            .event(envelope.data.kind.sse_event_name())
            .json_data(envelope)
            .expect("terminal event should serialize");
        Ok(sse)
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
}

fn terminal_events_from_persistent_history(
    state: &AppState,
    thread_id: Uuid,
    since: Option<u64>,
) -> anyhow::Result<Vec<TerminalEvent>> {
    let since = since.unwrap_or(0);
    let mut events = Vec::new();

    for history in state.store.list_terminal_history(thread_id, Some(since))? {
        let mut next_seq = history.seq_start;

        // Spawn failures contain only the terminal error event. Successful spawns
        // always reserve a start event and a distinct final event.
        if history.seq_start < history.seq_end {
            push_persistent_terminal_event(
                &mut events,
                since,
                &history,
                history.seq_start,
                history.started_at,
                TerminalEventKind::Started,
                TerminalEventFields {
                    command: Some(history.command.clone()),
                    cwd: history
                        .cwd
                        .as_ref()
                        .map(|path| path.to_string_lossy().to_string()),
                    ..Default::default()
                },
            );
            next_seq = history.seq_start.saturating_add(1);
        }

        if !history.stdout.is_empty() && next_seq < history.seq_end {
            push_persistent_terminal_event(
                &mut events,
                since,
                &history,
                next_seq,
                history.started_at,
                TerminalEventKind::Stdout,
                TerminalEventFields {
                    data: Some(history.stdout.clone()),
                    ..Default::default()
                },
            );
            next_seq = next_seq.saturating_add(1);
        }

        if !history.stderr.is_empty() && next_seq < history.seq_end {
            push_persistent_terminal_event(
                &mut events,
                since,
                &history,
                next_seq,
                history.started_at,
                TerminalEventKind::Stderr,
                TerminalEventFields {
                    data: Some(history.stderr.clone()),
                    ..Default::default()
                },
            );
        }

        let (kind, success) = match history.status {
            TerminalCommandStatus::Finished => (TerminalEventKind::Finished, Some(true)),
            TerminalCommandStatus::Failed => (TerminalEventKind::Finished, Some(false)),
            TerminalCommandStatus::Cancelled => (TerminalEventKind::Cancelled, Some(false)),
            TerminalCommandStatus::TimedOut | TerminalCommandStatus::Error => {
                (TerminalEventKind::Error, Some(false))
            }
        };
        push_persistent_terminal_event(
            &mut events,
            since,
            &history,
            history.seq_end,
            history.completed_at,
            kind,
            TerminalEventFields {
                command: (history.seq_start == history.seq_end).then(|| history.command.clone()),
                cwd: (history.seq_start == history.seq_end)
                    .then(|| {
                        history
                            .cwd
                            .as_ref()
                            .map(|path| path.to_string_lossy().to_string())
                    })
                    .flatten(),
                exit_code: history.exit_code,
                success,
                message: history.message.clone(),
                ..Default::default()
            },
        );
    }

    events.sort_by_key(|event| event.seq);
    Ok(events)
}

fn push_persistent_terminal_event(
    events: &mut Vec<TerminalEvent>,
    since: u64,
    history: &TerminalCommandHistory,
    seq: u64,
    created_at: DateTime<Utc>,
    kind: TerminalEventKind,
    fields: TerminalEventFields,
) {
    if seq <= since {
        return;
    }
    events.push(TerminalEvent {
        id: persistent_terminal_event_id(history.command_id, seq, kind),
        thread_id: history.thread_id,
        command_id: history.command_id,
        seq,
        created_at,
        kind,
        command: fields.command,
        cwd: fields.cwd,
        data: fields.data,
        exit_code: fields.exit_code,
        success: fields.success,
        message: fields.message,
    });
}

fn persistent_terminal_event_id(command_id: Uuid, seq: u64, kind: TerminalEventKind) -> Uuid {
    let mut bytes = *command_id.as_bytes();
    for (index, value) in seq.to_le_bytes().into_iter().enumerate() {
        bytes[8 + index] ^= value;
    }
    bytes[0] ^= match kind {
        TerminalEventKind::Started => 1,
        TerminalEventKind::Stdout => 2,
        TerminalEventKind::Stderr => 3,
        TerminalEventKind::Finished => 4,
        TerminalEventKind::Cancelled => 5,
        TerminalEventKind::Error => 6,
    };
    Uuid::from_bytes(bytes)
}

fn resolve_terminal_cwd(
    workspace_root: &FsPath,
    requested: Option<&FsPath>,
) -> Result<PathBuf, ApiError> {
    let root = canonical_workspace_root(workspace_root);
    let requested = requested.unwrap_or_else(|| FsPath::new("."));
    let requested = if requested.as_os_str().is_empty() {
        FsPath::new(".")
    } else {
        requested
    };
    if requested
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ApiError::bad_request("terminal cwd cannot contain .."));
    }

    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let resolved = candidate.canonicalize().map_err(|_| {
        ApiError::not_found(format!("terminal cwd not found: {}", candidate.display()))
    })?;
    if !resolved.starts_with(&root) {
        return Err(ApiError::bad_request(format!(
            "terminal cwd is outside workspace: {}",
            resolved.display()
        )));
    }
    if !resolved.is_dir() {
        return Err(ApiError::bad_request(format!(
            "terminal cwd is not a directory: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

#[derive(Debug, Deserialize)]
struct TerminalQuery {
    since: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalStartRequest {
    command: String,
    cwd: Option<PathBuf>,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TerminalStartResponse {
    thread_id: Uuid,
    command_id: Uuid,
    status: &'static str,
    history_url: String,
    stream_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalCancelRequest {
    command_id: Option<Uuid>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TerminalCancelResponse {
    command_id: Option<Uuid>,
    cancelled: bool,
    message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSessionCreateRequest {
    cwd: Option<PathBuf>,
    cols: Option<u16>,
    rows: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSessionInputRequest {
    session_id: Uuid,
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSessionResizeRequest {
    session_id: Uuid,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSessionCloseRequest {
    session_id: Uuid,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TerminalSessionResponse {
    session_id: Uuid,
    thread_id: Uuid,
    status: &'static str,
    cwd: PathBuf,
    shell: String,
    process_id: Option<u32>,
    started_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn terminal_bus_replays_only_events_after_cursor() {
        let bus = TerminalBus::default();
        let thread_id = Uuid::new_v4();
        let command_id = Uuid::new_v4();

        let first = bus.publish_event(
            thread_id,
            command_id,
            TerminalEventKind::Started,
            TerminalEventFields::default(),
        );
        let second = bus.publish_event(
            thread_id,
            command_id,
            TerminalEventKind::Stdout,
            TerminalEventFields {
                data: Some("ready".to_string()),
                ..Default::default()
            },
        );

        assert_eq!(second.seq, first.seq + 1);
        assert_eq!(bus.history(thread_id, Some(first.seq)), vec![second]);
    }

    #[test]
    fn terminal_cwd_rejects_parent_traversal() {
        let error = resolve_terminal_cwd(FsPath::new("."), Some(FsPath::new("../outside")))
            .expect_err("parent traversal must be rejected");

        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("cannot contain .."));
    }

    #[test]
    fn terminal_session_request_keeps_camel_case_contract() {
        let request: TerminalSessionResizeRequest = serde_json::from_value(serde_json::json!({
            "sessionId": Uuid::nil(),
            "cols": 120,
            "rows": 40
        }))
        .expect("deserialize terminal resize request");

        assert_eq!(request.session_id, Uuid::nil());
        assert_eq!(request.cols, 120);
        assert_eq!(request.rows, 40);
    }
}
