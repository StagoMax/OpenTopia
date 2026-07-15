use anyhow::Context;
use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use clap::Parser;
use futures_util::stream::{self, StreamExt};
use opentopia_core::mcp_host::McpExtensionHost;
use opentopia_core::{
    browser_domain_approval_action, browser_domain_from_approval_action, browser_domain_from_url,
    browser_domain_is_approved, build_local_sandbox_command, discover_skills, execute_git_workflow,
    load_context_sources, AgentContextBudget, AgentContinuation, AgentCore, AgentEvent,
    AgentEventPayload, AgentTurnInput, AgentTurnOutcome, AppSettings, Approval, ApprovalStatus,
    Artifact, ArtifactMetadata, BasicPolicyEngine, BrowserDownloadRequest, BrowserNavigateRequest,
    BrowserOutput, BrowserRuntime, BrowserRuntimeConfig, BrowserSelector, BrowserSessionId,
    BrowserTypeRequest, BrowserWaitCondition, BrowserWaitRequest, ChangedFile, ContextSourcePolicy,
    ContextSourceRef, ContextSummary, ExecRequest, ExecutionContext, GitWorkflowAction,
    GitWorkflowRequest, LocalBrowserRuntime, LocalExecutionEnvironment, McpCallResult,
    McpServerConfig, McpServerStatus, McpToolDescriptor, Message, MessagePart, MessageRole,
    ModelContentPart, ModelConversationMessage, ModelConversationRole, ModelProvider, ModelRequest,
    OpenAiCompatibleProvider, PermissionMode, PolicyDecision, PolicyEngine, ProviderHealth,
    ProviderHealthCheck, ProviderKind, ProviderSettings, ResourceLimit, SandboxDescriptor,
    SandboxSettings, SessionStore, SkillDescriptor, SkillRef, SpawnSubagentRequest,
    SqliteSessionStore, StoreError, SubagentExecutor, SubagentObserver, SubagentRun,
    SubagentScheduler, SubagentSchedulerConfig, TerminalCommandHistory, TerminalCommandStatus,
    ThreadMcpServer, ToolCall, ToolPermissionDescriptor, ToolResult, WorkspaceDiff,
    WorkspaceDiffHunk, WorkspaceDiffScope, WorkspaceEntry, WorkspaceEntryKind,
    WorkspaceFilePreview, WorkspaceTree,
};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::timeout;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use uuid::Uuid;

mod auth;
mod turns;

use auth::ApiAuth;
use turns::{TurnCancelResult, TurnHandle, TurnManager, TurnStatus};

#[derive(Debug, Parser)]
#[command(name = "opentopia-server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8787)]
    port: u16,
    #[arg(long, env = "OPENTOPIA_DB", default_value = ".opentopia/opentopia.db")]
    db: PathBuf,
    #[arg(long, env = "OPENTOPIA_PERMISSION", default_value = "auto")]
    permission: PermissionMode,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "opentopia_server=info,tower_http=info".into()),
        )
        .init();

    let args = Args::parse();
    let auth = ApiAuth::from_env()?;
    let store = Arc::new(SqliteSessionStore::open(&args.db)?);
    let interrupted_subagents = store.fail_interrupted_subagent_runs()?;
    if interrupted_subagents > 0 {
        info!(interrupted_subagents, "recovered interrupted subagent runs");
    }
    let loaded_settings = store.load_settings(args.permission)?;
    let settings = Arc::new(RwLock::new(loaded_settings.clone()));
    let mcp_settings = settings.clone();
    let mcp_host = McpExtensionHost::with_execution_environment_factory(move |config| {
        let workspace_root = config
            .cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let sandbox_config = mcp_settings
            .read()
            .expect("settings lock poisoned")
            .sandbox
            .to_local_sandbox_config();
        Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            workspace_root,
            sandbox_config,
        ))
    });
    let browser = Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default()));
    let mut initial_agent = AgentCore::from_settings(&loaded_settings);
    initial_agent.set_browser_runtime(browser.clone());
    let agent = Arc::new(RwLock::new(initial_agent));
    let subagents = SubagentScheduler::new(
        SubagentSchedulerConfig::default(),
        Arc::new(ServerSubagentExecutor {
            store: store.clone(),
            agent: agent.clone(),
            settings: settings.clone(),
            mcp_host: mcp_host.clone(),
        }),
        Arc::new(StoreSubagentObserver {
            store: store.clone(),
        }),
    );
    agent
        .write()
        .expect("agent lock poisoned")
        .set_subagent_scheduler(subagents.clone());
    let state = AppState {
        store,
        agent,
        settings,
        events: EventBus::default(),
        terminals: TerminalBus::default(),
        ptys: PtyManager::default(),
        browser,
        mcp_host,
        auth,
        turns: TurnManager::default(),
        subagents,
    };

    let event_state = state.clone();
    let mut subagent_events = state.subagents.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = subagent_events.recv().await {
            publish_payload(
                &event_state,
                event.run.parent_thread_id,
                Some(event.run.parent_turn_id),
                AgentEventPayload::SubagentUpdated { run: event.run },
            );
        }
    });

    let app = build_router(state);
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, db = %args.db.display(), "OpenTopia server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

fn build_router(state: AppState) -> Router {
    let cors = state.auth.cors_layer();
    let auth_state = state.clone();
    Router::new()
        .route("/health", get(health))
        .route("/api/settings", get(get_settings).patch(update_settings))
        .route("/api/skills", get(list_skills))
        .route("/api/provider/health", get(provider_health))
        .route("/api/provider/test", post(test_provider_connection))
        .route("/api/threads", get(list_threads).post(create_thread))
        .route(
            "/api/threads/:thread_id",
            patch(update_thread).delete(delete_thread),
        )
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/:project_id",
            patch(update_project).delete(delete_project),
        )
        .route(
            "/api/threads/:thread_id/messages",
            get(list_messages).post(send_message),
        )
        .route("/api/threads/:thread_id/events", get(list_events))
        .route("/api/threads/:thread_id/events/stream", get(stream_events))
        .route("/api/threads/:thread_id/turn", get(get_turn_status))
        .route(
            "/api/threads/:thread_id/subagents",
            get(list_subagent_runs).post(spawn_subagent_run),
        )
        .route(
            "/api/threads/:thread_id/subagents/:run_id/input",
            post(send_subagent_input),
        )
        .route(
            "/api/threads/:thread_id/subagents/:run_id/cancel",
            post(cancel_subagent_run),
        )
        .route(
            "/api/threads/:thread_id/subagents/:run_id/wait",
            post(wait_subagent_run),
        )
        .route(
            "/api/threads/:thread_id/turn/cancel",
            post(cancel_agent_turn),
        )
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
        .route(
            "/api/threads/:thread_id/workspace/tree",
            get(list_workspace_tree),
        )
        .route(
            "/api/threads/:thread_id/workspace/file",
            get(read_workspace_file),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff",
            get(get_workspace_diff),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff/revert",
            post(revert_workspace_file),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff/hunk",
            post(apply_workspace_diff_hunk),
        )
        .route("/api/threads/:thread_id/sandbox", get(get_sandbox))
        .route("/api/threads/:thread_id/browser", post(run_browser_command))
        .route("/api/threads/:thread_id/git", post(run_git_workflow))
        .route("/api/threads/:thread_id/context", get(get_context_status))
        .route(
            "/api/threads/:thread_id/context/compact",
            post(compact_context),
        )
        .route("/api/threads/:thread_id/trajectory", get(export_trajectory))
        .route("/api/threads/:thread_id/artifacts", get(list_artifacts))
        .route(
            "/api/threads/:thread_id/artifacts/:artifact_id",
            get(get_artifact),
        )
        .route("/api/threads/:thread_id/approvals", get(list_approvals))
        .route(
            "/api/threads/:thread_id/approvals/:approval_id/decision",
            post(decide_approval),
        )
        .route(
            "/api/mcp/servers",
            get(list_mcp_servers).post(create_mcp_server),
        )
        .route(
            "/api/mcp/servers/:server_id",
            patch(update_mcp_server).delete(delete_mcp_server),
        )
        .route(
            "/api/mcp/servers/:server_id/restart",
            post(restart_mcp_server),
        )
        .route("/api/mcp/servers/:server_id/tools", get(list_mcp_tools))
        .route("/api/mcp/servers/:server_id/call-tool", post(call_mcp_tool))
        .route("/api/threads/:thread_id/mcp", get(list_thread_mcp_servers))
        .route(
            "/api/threads/:thread_id/mcp/:server_id",
            put(set_thread_mcp_server),
        )
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            auth::authorize,
        ))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Clone)]
struct AppState {
    store: Arc<SqliteSessionStore>,
    agent: Arc<RwLock<AgentCore>>,
    settings: Arc<RwLock<AppSettings>>,
    events: EventBus,
    terminals: TerminalBus,
    ptys: PtyManager,
    browser: Arc<LocalBrowserRuntime>,
    mcp_host: McpExtensionHost,
    auth: ApiAuth,
    turns: TurnManager,
    subagents: SubagentScheduler,
}

struct StoreSubagentObserver {
    store: Arc<SqliteSessionStore>,
}

impl SubagentObserver for StoreSubagentObserver {
    fn on_update(&self, run: &SubagentRun) {
        if let Err(error) = self.store.upsert_subagent_run(run) {
            error!(?error, run_id = %run.id, "failed to persist subagent run");
        }
    }
}

struct ServerSubagentExecutor {
    store: Arc<SqliteSessionStore>,
    agent: Arc<RwLock<AgentCore>>,
    settings: Arc<RwLock<AppSettings>>,
    mcp_host: McpExtensionHost,
}

#[async_trait]
impl SubagentExecutor for ServerSubagentExecutor {
    async fn execute(
        &self,
        run: SubagentRun,
        mut input: mpsc::UnboundedReceiver<String>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<String> {
        let thread = self
            .store
            .get_thread(run.parent_thread_id)?
            .ok_or_else(|| anyhow::anyhow!("parent thread no longer exists"))?;
        let mut conversation = Vec::new();
        let mut prompt = run.input.clone();
        loop {
            while let Ok(extra) = input.try_recv() {
                prompt.push_str("\n\nAdditional parent input:\n");
                prompt.push_str(&extra);
            }
            let settings = self
                .settings
                .read()
                .expect("settings lock poisoned")
                .clone();
            let mut agent = self.agent.read().expect("agent lock poisoned").clone();
            agent.set_mcp_host(self.mcp_host.clone());
            agent.set_subagent_context(run.id, run.depth);
            sync_thread_mcp_tools(&self.store, run.parent_thread_id, &mut agent).await;
            let result = agent
                .run_turn_detailed_streaming(
                    AgentTurnInput {
                        thread_id: run.parent_thread_id,
                        user_message_id: Uuid::new_v4(),
                        workspace_root: thread.workspace_root.clone(),
                        content: prompt.clone(),
                        user_content: Vec::new(),
                        context_summary: None,
                        conversation: conversation.clone(),
                        permission_mode: settings.permission_mode,
                        context_budget: None,
                        store: Some(self.store.clone()),
                        cancellation: Some(cancellation.clone()),
                    },
                    None,
                )
                .await?;
            if matches!(result.outcome, AgentTurnOutcome::Suspended { .. }) {
                anyhow::bail!(
                    "subagent requires approval; the parent must perform this action directly"
                );
            }
            let last_result = subagent_result_text(&result.events);
            conversation.push(ModelConversationMessage {
                role: ModelConversationRole::User,
                content: prompt,
                content_parts: Vec::new(),
            });
            conversation.push(ModelConversationMessage {
                role: ModelConversationRole::Assistant,
                content: last_result.clone(),
                content_parts: Vec::new(),
            });

            let follow_up = match timeout(Duration::from_millis(25), input.recv()).await {
                Ok(Some(follow_up)) => follow_up,
                _ => return Ok(last_result),
            };
            prompt = follow_up;
        }
    }
}

fn subagent_result_text(events: &[AgentEventPayload]) -> String {
    let messages = events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::AssistantMessage { message } => Some(message),
            _ => None,
        })
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part {
            MessagePart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        events
            .iter()
            .rev()
            .find_map(|event| match event {
                AgentEventPayload::TurnFinished { summary } => Some(summary.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "Subagent completed without a text result.".to_string())
    } else {
        messages.join("\n\n")
    }
}

#[derive(Clone, Default)]
struct EventBus {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<AgentEvent>>>>,
}

impl EventBus {
    fn subscribe(&self, thread_id: Uuid) -> broadcast::Receiver<AgentEvent> {
        let mut channels = self.channels.write().expect("event bus poisoned");
        channels
            .entry(thread_id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(256);
                tx
            })
            .subscribe()
    }

    fn publish(&self, event: AgentEvent) {
        let sender = {
            let mut channels = self.channels.write().expect("event bus poisoned");
            channels
                .entry(event.thread_id)
                .or_insert_with(|| {
                    let (tx, _rx) = broadcast::channel(256);
                    tx
                })
                .clone()
        };
        let _ = sender.send(event);
    }
}

const TERMINAL_HISTORY_LIMIT: usize = 2_000;
const DEFAULT_TERMINAL_TIMEOUT_MS: u64 = 300_000;
const TERMINAL_OUTPUT_BYTES_LIMIT: usize = 4 * 1024 * 1024;
const GIT_OUTPUT_BYTES_LIMIT: usize = 8 * 1024 * 1024;
const SENSITIVE_CHILD_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "OPENTOPIA_API_KEY",
    "OPENTOPIA_API_TOKEN",
    "CREDIT_REVIEW_LLM_API_KEY",
];
const MAX_TERMINAL_TIMEOUT_MS: u64 = 3_600_000;

#[derive(Clone, Default)]
struct TerminalBus {
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
    fn subscribe(&self, thread_id: Uuid) -> broadcast::Receiver<TerminalEvent> {
        let mut channels = self.channels.write().expect("terminal bus poisoned");
        channels
            .entry(thread_id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(512);
                tx
            })
            .subscribe()
    }

    fn history(&self, thread_id: Uuid, since: Option<u64>) -> Vec<TerminalEvent> {
        let histories = self.histories.read().expect("terminal history poisoned");
        histories
            .get(&thread_id)
            .into_iter()
            .flatten()
            .filter(|event| since.map_or(true, |seq| event.seq > seq))
            .cloned()
            .collect()
    }

    fn ensure_min_seq(&self, thread_id: Uuid, min_seq: u64) {
        let mut next_seq = self.next_seq.write().expect("terminal seq poisoned");
        let entry = next_seq.entry(thread_id).or_insert(0);
        if *entry < min_seq {
            *entry = min_seq;
        }
    }

    fn register_running(
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

    fn cancel_running(
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

    fn remove_running(&self, thread_id: Uuid, command_id: Uuid) {
        let mut running = self.running.write().expect("terminal running poisoned");
        if running
            .get(&thread_id)
            .is_some_and(|active| active.command_id == command_id)
        {
            running.remove(&thread_id);
        }
    }

    fn publish_event(
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
struct PtyManager {
    sessions: Arc<RwLock<HashMap<Uuid, Arc<PtySession>>>>,
}

struct PtySession {
    session_id: Uuid,
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
    fn get(&self, thread_id: Uuid) -> Option<Arc<PtySession>> {
        self.sessions
            .read()
            .expect("pty sessions poisoned")
            .get(&thread_id)
            .filter(|session| session.running.load(Ordering::SeqCst))
            .cloned()
    }

    fn insert(&self, session: Arc<PtySession>) {
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
    fn view(&self) -> TerminalSessionResponse {
        TerminalSessionResponse {
            session_id: self.session_id,
            thread_id: self.thread_id,
            status: if self.running.load(Ordering::SeqCst) {
                "running"
            } else {
                "closed"
            },
            cwd: self.cwd.clone(),
            shell: self.shell.clone(),
            process_id: self.process_id,
            started_at: self.started_at,
        }
    }

    fn write(&self, data: &str) -> anyhow::Result<()> {
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

    fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
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

    fn kill(&self) -> anyhow::Result<()> {
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

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        service: "opentopia-server",
        api_version: 1,
    })
}

async fn get_settings(State(state): State<AppState>) -> Json<AppSettings> {
    Json(current_settings(&state))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(request): Json<SettingsPatchRequest>,
) -> Result<Json<AppSettings>, ApiError> {
    let mut settings = current_settings(&state);
    if let Some(providers) = request.providers {
        settings.providers = providers;
    }
    if let Some(active_provider_id) = request.active_provider_id {
        settings.active_provider_id = active_provider_id;
    }
    if let Some(kind) = request.provider_kind {
        settings.active_provider_mut().kind = kind;
    }
    if let Some(base_url) = request.base_url {
        let base_url = base_url.trim();
        if base_url.is_empty() {
            return Err(ApiError::bad_request("baseUrl cannot be empty"));
        }
        settings.active_provider_mut().base_url = base_url.to_string();
    }
    if let Some(model) = request.model {
        let model = model.trim();
        if model.is_empty() {
            return Err(ApiError::bad_request("model cannot be empty"));
        }
        settings.active_provider_mut().model = model.to_string();
    }
    if let Some(api_key_source) = request.api_key_source {
        let api_key_source = api_key_source.trim();
        if api_key_source.is_empty() {
            return Err(ApiError::bad_request("apiKeySource cannot be empty"));
        }
        settings.active_provider_mut().api_key_source = api_key_source.to_string();
    }
    if let Some(permission_mode) = request.permission_mode {
        settings.permission_mode = permission_mode;
    }
    if request.clear_default_workspace_root.unwrap_or(false) {
        settings.default_workspace_root = None;
    } else if let Some(default_workspace_root) = request.default_workspace_root {
        settings.default_workspace_root = Some(default_workspace_root);
    }
    if let Some(sandbox) = request.sandbox {
        settings.sandbox = sandbox;
    }

    let settings = state.store.save_settings(settings)?;
    {
        let mut settings_guard = state.settings.write().expect("settings lock poisoned");
        *settings_guard = settings.clone();
    }
    {
        let mut agent_guard = state.agent.write().expect("agent lock poisoned");
        let mut agent = AgentCore::from_settings(&settings);
        agent.set_browser_runtime(state.browser.clone());
        agent.set_subagent_scheduler(state.subagents.clone());
        *agent_guard = agent;
    }
    Ok(Json(settings))
}

async fn provider_health(State(state): State<AppState>) -> Json<Vec<ProviderHealth>> {
    let settings = current_settings(&state);
    Json(
        settings
            .providers
            .iter()
            .map(ProviderHealth::from_settings)
            .collect(),
    )
}

async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillsQuery>,
) -> Result<Json<Vec<SkillDescriptor>>, ApiError> {
    let workspace_root = match query.workspace_root {
        Some(workspace_root) => {
            if state
                .store
                .find_project_by_workspace(&workspace_root)?
                .is_none()
            {
                return Err(ApiError::bad_request(
                    "workspace is not registered as a project",
                ));
            }
            Some(workspace_root)
        }
        None => None,
    };
    Ok(Json(discover_skills(workspace_root.as_deref())))
}

async fn test_provider_connection(
    State(state): State<AppState>,
    Json(request): Json<ProviderTestRequest>,
) -> Result<Json<ProviderHealthCheck>, ApiError> {
    let settings = current_settings(&state);
    let provider_settings = if let Some(provider_id) = &request.provider_id {
        settings
            .providers
            .iter()
            .find(|p| &p.id == provider_id)
            .ok_or_else(|| ApiError::not_found(format!("provider not found: {provider_id}")))?
    } else {
        settings.active_provider()
    };
    let provider = OpenAiCompatibleProvider::from_settings(provider_settings)
        .ok_or_else(|| ApiError::bad_request("provider is not an OpenAI-compatible provider"))?;
    let result = provider.check_health().await?;
    Ok(Json(result))
}

async fn list_threads(
    State(state): State<AppState>,
    Query(query): Query<ThreadListQuery>,
) -> Result<Json<Vec<opentopia_core::Thread>>, ApiError> {
    Ok(Json(
        state
            .store
            .list_threads_including_archived(query.include_archived)?,
    ))
}

async fn create_thread(
    State(state): State<AppState>,
    Json(request): Json<CreateThreadRequest>,
) -> Result<Json<opentopia_core::Thread>, ApiError> {
    let thread = if let Some(project_id) = request.project_id {
        state
            .store
            .create_thread_in_project(request.title, project_id)?
    } else if let Some(workspace_root) = request.workspace_root {
        let workspace_root = canonicalize_workspace_root(workspace_root);
        let project = state
            .store
            .find_or_create_project(project_name_for_workspace(&workspace_root), workspace_root)?;
        state
            .store
            .create_thread_in_project(request.title, project.id)?
    } else {
        let workspace_root = std::env::current_dir().map_err(anyhow::Error::from)?;
        state.store.create_thread(request.title, workspace_root)?
    };
    Ok(Json(thread))
}

async fn update_thread(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<UpdateThreadRequest>,
) -> Result<Json<opentopia_core::Thread>, ApiError> {
    let archived = request.archived.or_else(|| match request.archived_at {
        PatchValue::Missing => None,
        PatchValue::Null => Some(false),
        PatchValue::Value(_) => Some(true),
    });
    let project_id = match request.project_id {
        PatchValue::Missing => None,
        PatchValue::Null => Some(None),
        PatchValue::Value(project_id) => Some(Some(project_id)),
    };
    let thread = state
        .store
        .update_thread(thread_id, request.title, project_id, archived)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    Ok(Json(thread))
}

async fn delete_thread(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let deleted = state.store.delete_thread(thread_id)?;
    if !deleted {
        return Err(ApiError::not_found(format!(
            "thread not found: {thread_id}"
        )));
    }
    Ok(Json(DeleteResponse { deleted }))
}

async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<opentopia_core::Project>>, ApiError> {
    Ok(Json(state.store.list_projects()?))
}

async fn create_project(
    State(state): State<AppState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<Json<opentopia_core::Project>, ApiError> {
    let workspace_root = request.workspace_root.map(canonicalize_workspace_root);
    let project = state.store.create_project(
        request.name,
        workspace_root,
        request.pinned.unwrap_or(false),
        request.sort_order.unwrap_or(0),
    )?;
    Ok(Json(project))
}

async fn update_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(request): Json<UpdateProjectRequest>,
) -> Result<Json<opentopia_core::Project>, ApiError> {
    let workspace_root = match request.workspace_root {
        PatchValue::Missing => None,
        PatchValue::Null => Some(None),
        PatchValue::Value(path) => Some(Some(canonicalize_workspace_root(path))),
    };
    let project = state
        .store
        .update_project(
            project_id,
            request.name,
            workspace_root,
            request.pinned,
            request.sort_order,
        )?
        .ok_or_else(|| ApiError::not_found(format!("project not found: {project_id}")))?;
    Ok(Json(project))
}

async fn delete_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, ApiError> {
    let deleted = state.store.delete_project(project_id)?;
    if !deleted {
        return Err(ApiError::not_found(format!(
            "project not found: {project_id}"
        )));
    }
    Ok(Json(DeleteResponse { deleted }))
}

fn canonicalize_workspace_root(workspace_root: PathBuf) -> PathBuf {
    workspace_root.canonicalize().unwrap_or(workspace_root)
}

fn project_name_for_workspace(workspace_root: &FsPath) -> String {
    workspace_root
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .filter(|part| *part != ".")
        .unwrap_or("Workspace")
        .to_string()
}

async fn list_messages(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<Message>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_messages(thread_id)?))
}

async fn send_message(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<Message>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    if let Some(command) = legacy_direct_tool_command(&request.content) {
        return Err(ApiError::bad_request(format!(
            "{command} is a direct tool command. Use the terminal or file workspace API instead of sending it to the agent."
        )));
    }
    if request.content.trim().is_empty()
        && request.source_paths.is_empty()
        && request.skill_ids.is_empty()
    {
        return Err(ApiError::bad_request("message content cannot be empty"));
    }

    let sources = load_context_sources(&request.source_paths, &ContextSourcePolicy::default())
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    // Skills are pinned user context, not an instruction pipeline. The model can inspect the
    // catalog and read a Skill through tools when it decides that the task calls for one.
    let skill_catalog = discover_skills(Some(&thread.workspace_root))
        .into_iter()
        .map(|skill| (skill.id.clone(), skill))
        .collect::<HashMap<_, _>>();
    let mut pinned_skills = Vec::new();
    let mut seen_skill_ids = HashSet::new();
    for skill_id in &request.skill_ids {
        if !seen_skill_ids.insert(skill_id) {
            continue;
        }
        let skill = skill_catalog.get(skill_id).ok_or_else(|| {
            ApiError::bad_request(format!("unknown or unavailable skill: {skill_id}"))
        })?;
        pinned_skills.push(SkillRef {
            id: skill.id.clone(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            path: skill.path.clone(),
            truncated: false,
        });
    }
    let prompt = if request.content.trim().is_empty() {
        "Review the attached sources.".to_string()
    } else {
        request.content.clone()
    };

    if !state
        .store
        .list_approvals(thread_id, Some(ApprovalStatus::Pending))?
        .is_empty()
    {
        return Err(ApiError::conflict(
            "resolve the pending approval before starting another turn",
        ));
    }

    let mut pending_message = Message::text(thread_id, MessageRole::User, prompt.clone());
    pending_message
        .parts
        .extend(sources.iter().map(|source| MessagePart::SourceRef {
            source: ContextSourceRef::from(source),
        }));
    pending_message.parts.extend(
        pinned_skills
            .into_iter()
            .map(|skill| MessagePart::SkillRef { skill }),
    );
    let turn = state
        .turns
        .begin(thread_id, pending_message.id)
        .map_err(|active| {
            ApiError::conflict(format!("thread already has active turn {}", active.turn_id))
        })?;
    let user_message = match state.store.append_message(pending_message) {
        Ok(message) => message,
        Err(err) => {
            state.turns.finish(thread_id, turn.turn_id);
            return Err(err.into());
        }
    };

    let run_state = state.clone();
    let run_message = user_message.clone();
    let model_content = prompt;
    let model_user_content = sources
        .iter()
        .flat_map(|source| source.content_or_legacy_text())
        .collect::<Vec<_>>();
    tokio::spawn(async move {
        run_new_agent_turn(
            run_state,
            thread,
            run_message,
            model_content,
            model_user_content,
            turn,
        )
        .await;
    });

    Ok(Json(user_message))
}

fn legacy_direct_tool_command(content: &str) -> Option<&'static str> {
    match content.trim().split_whitespace().next()? {
        command if command.eq_ignore_ascii_case("/run") => Some("/run"),
        command if command.eq_ignore_ascii_case("/read") => Some("/read"),
        _ => None,
    }
}

async fn decide_approval(
    State(state): State<AppState>,
    Path((thread_id, approval_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ApprovalDecisionRequest>,
) -> Result<Json<ApprovalDecisionResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let pending = state
        .store
        .get_approval(approval_id)?
        .ok_or_else(|| ApiError::not_found(format!("approval not found: {approval_id}")))?;
    if pending.thread_id != thread_id {
        return Err(ApiError::bad_request(
            "approval does not belong to this thread",
        ));
    }
    if pending.status != ApprovalStatus::Pending {
        return Err(ApiError::conflict(format!(
            "approval already decided: {approval_id}"
        )));
    }

    if browser_domain_from_approval_action(&pending.action).is_some() {
        let status = if request.approved {
            ApprovalStatus::Approved
        } else {
            ApprovalStatus::Denied
        };
        state
            .store
            .update_approval_status(approval_id, status)?
            .ok_or_else(|| ApiError::not_found(format!("approval not found: {approval_id}")))?;
        return Ok(Json(ApprovalDecisionResponse {
            accepted: true,
            // Browser-panel commands are intentionally not replayed implicitly. The approved
            // domain grant lets the user or model make the next explicit navigation.
            executed: false,
        }));
    }

    let continuation_value = state
        .store
        .get_approval_continuation(approval_id, thread_id)?
        .ok_or_else(|| ApiError::conflict("approval continuation is not available"))?;
    let continuation: AgentContinuation = serde_json::from_value(continuation_value)
        .map_err(|err| ApiError::internal(format!("invalid approval continuation: {err}")))?;
    let turn = state
        .turns
        .begin(thread_id, continuation.user_message_id)
        .map_err(|active| {
            ApiError::conflict(format!("thread already has active turn {}", active.turn_id))
        })?;
    let status = if request.approved {
        ApprovalStatus::Approved
    } else {
        ApprovalStatus::Denied
    };
    state
        .store
        .update_approval_status(approval_id, status)?
        .ok_or_else(|| ApiError::not_found(format!("approval not found: {approval_id}")))?;
    let run_state = state.clone();
    tokio::spawn(async move {
        run_resumed_agent_turn(run_state, approval_id, continuation, request.approved, turn).await;
    });

    Ok(Json(ApprovalDecisionResponse {
        accepted: true,
        executed: request.approved,
    }))
}

async fn get_turn_status(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<TurnStatus>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.turns.status(thread_id)))
}

async fn list_subagent_runs(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<SubagentRun>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_subagent_runs(thread_id)?))
}

async fn spawn_subagent_run(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<SpawnSubagentRunRequest>,
) -> Result<Json<SubagentRun>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let parent_turn_id = request
        .parent_turn_id
        .or_else(|| state.turns.status(thread_id).map(|turn| turn.turn_id))
        .unwrap_or_else(Uuid::new_v4);
    let run = state
        .subagents
        .spawn(SpawnSubagentRequest {
            parent_thread_id: thread_id,
            parent_turn_id,
            name: request.name,
            input: request.input,
            depth: request.depth.unwrap_or(1),
        })
        .map_err(subagent_api_error)?;
    Ok(Json(run))
}

async fn send_subagent_input(
    State(state): State<AppState>,
    Path((thread_id, run_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<SubagentInputRequest>,
) -> Result<StatusCode, ApiError> {
    ensure_live_subagent(&state, thread_id, run_id)?;
    state
        .subagents
        .send_input(run_id, request.input)
        .map_err(subagent_api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_subagent_run(
    State(state): State<AppState>,
    Path((thread_id, run_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    ensure_live_subagent(&state, thread_id, run_id)?;
    state.subagents.cancel(run_id).map_err(subagent_api_error)?;
    Ok(StatusCode::ACCEPTED)
}

async fn wait_subagent_run(
    State(state): State<AppState>,
    Path((thread_id, run_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<WaitSubagentRunRequest>,
) -> Result<Json<SubagentRun>, ApiError> {
    ensure_live_subagent(&state, thread_id, run_id)?;
    let wait_timeout =
        Duration::from_millis(request.timeout_ms.unwrap_or(30_000).clamp(1, 120_000));
    Ok(Json(
        state
            .subagents
            .wait(run_id, wait_timeout)
            .await
            .map_err(subagent_api_error)?,
    ))
}

fn ensure_live_subagent(
    state: &AppState,
    thread_id: Uuid,
    run_id: Uuid,
) -> Result<SubagentRun, ApiError> {
    ensure_thread(state, thread_id)?;
    let run = state
        .subagents
        .get(run_id)
        .ok_or_else(|| ApiError::not_found(format!("active subagent run not found: {run_id}")))?;
    if run.parent_thread_id != thread_id {
        return Err(ApiError::bad_request(
            "subagent run does not belong to this thread",
        ));
    }
    Ok(run)
}

fn subagent_api_error(error: opentopia_core::SubagentError) -> ApiError {
    match error {
        opentopia_core::SubagentError::NotFound(_) => ApiError::not_found(error.to_string()),
        opentopia_core::SubagentError::AlreadyTerminal(_)
        | opentopia_core::SubagentError::InputClosed(_) => ApiError::conflict(error.to_string()),
        _ => ApiError::bad_request(error.to_string()),
    }
}

async fn cancel_agent_turn(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<CancelAgentTurnRequest>,
) -> Result<Json<TurnCancelResult>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let parent_turn_id = request
        .turn_id
        .or_else(|| state.turns.status(thread_id).map(|turn| turn.turn_id));
    let result = state.turns.cancel(thread_id, request.turn_id);
    if result.cancelled {
        if let Some(parent_turn_id) = parent_turn_id {
            state.subagents.cancel_parent(parent_turn_id);
        }
    }
    Ok(Json(result))
}

async fn list_approvals(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<ApprovalQuery>,
) -> Result<Json<Vec<Approval>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_approvals(thread_id, query.status)?))
}

async fn list_artifacts(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<ArtifactMetadata>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_artifacts(thread_id)?))
}

async fn get_artifact(
    State(state): State<AppState>,
    Path((thread_id, artifact_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Artifact>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let artifact = state
        .store
        .get_artifact(thread_id, artifact_id)?
        .ok_or_else(|| ApiError::not_found(format!("artifact not found: {artifact_id}")))?;
    Ok(Json(artifact))
}

async fn list_workspace_tree(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceTree>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let path = resolve_workspace_path(&root, query.path.as_deref())?;
    let entries = list_workspace_entries(&root, &path)?;
    Ok(Json(WorkspaceTree {
        root,
        path: relative_workspace_path(&thread.workspace_root, &path),
        entries,
    }))
}

async fn read_workspace_file(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceFilePreview>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let path = resolve_workspace_path(&root, query.path.as_deref())?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| ApiError::not_found(format!("file not found: {}", path.display())))?;
    if !metadata.is_file() {
        return Err(ApiError::bad_request(format!(
            "path is not a file: {}",
            path.display()
        )));
    }

    let bytes = tokio::fs::read(&path).await?;
    let content = String::from_utf8_lossy(&bytes);
    let (content, truncated) = truncate_with_flag(&content, 64_000);
    Ok(Json(WorkspaceFilePreview {
        path: relative_workspace_path(&root, &path),
        content,
        bytes: bytes.len(),
        truncated,
        readonly: true,
    }))
}

async fn get_workspace_diff(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<WorkspaceDiff>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let diff = get_workspace_diff_inner(&thread.workspace_root).await?;
    Ok(Json(diff))
}

async fn revert_workspace_file(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<WorkspaceDiffRevertRequest>,
) -> Result<Json<WorkspaceDiffActionResponse>, ApiError> {
    if !request.confirm {
        return Err(ApiError::bad_request(
            "confirm must be true to revert a workspace file",
        ));
    }
    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let relative_path = validate_relative_git_path(&request.path)?;

    let status_output = run_git(
        &root,
        ["status", "--porcelain=v1", "--", relative_path.as_str()],
    )
    .await?;
    if status_output.trim().is_empty() {
        return Err(ApiError::bad_request(format!(
            "no working-tree change found for {}",
            relative_path
        )));
    }
    let status_files = parse_git_status(&status_output);
    let changed_file = status_files
        .iter()
        .find(|file| normalized_path_string(&file.path) == relative_path)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "no working-tree change found for {}",
                relative_path
            ))
        })?;
    if changed_file.is_untracked {
        return Err(ApiError::bad_request(
            "untracked files are not reverted by this safe action",
        ));
    }
    if changed_file.is_renamed {
        return Err(ApiError::bad_request(
            "renamed paths must be reverted manually for now",
        ));
    }
    if !changed_file.staged_status.is_empty() {
        return Err(ApiError::bad_request(
            "files with staged changes must be handled manually before worktree restore",
        ));
    }
    if !matches!(
        changed_file.unstaged_status.as_str(),
        "modified" | "deleted"
    ) {
        return Err(ApiError::bad_request(
            "only unstaged modified or deleted tracked files can be restored",
        ));
    }

    run_git(
        &root,
        ["ls-files", "--error-unmatch", "--", relative_path.as_str()],
    )
    .await?;
    run_git(
        &root,
        [
            "restore",
            "--source=HEAD",
            "--worktree",
            "--",
            relative_path.as_str(),
        ],
    )
    .await?;
    let diff = get_workspace_diff_inner(&root).await?;
    Ok(Json(WorkspaceDiffActionResponse {
        path: PathBuf::from(relative_path),
        diff,
    }))
}

async fn apply_workspace_diff_hunk(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<WorkspaceDiffHunkActionRequest>,
) -> Result<Json<WorkspaceDiffActionResponse>, ApiError> {
    if !request.confirm {
        return Err(ApiError::bad_request(
            "confirm must be true to change a workspace diff hunk",
        ));
    }
    if request.patch.len() > 100_000 {
        return Err(ApiError::bad_request("hunk patch is too large"));
    }

    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let relative_path = validate_relative_git_path(&request.path)?;
    let current_diff = get_workspace_diff_inner(&root).await?;
    let current_hunk = current_diff.hunks.iter().find(|hunk| {
        normalized_path_string(&hunk.path) == relative_path
            && hunk.scope == request.scope
            && hunk.patch == request.patch
    });
    if current_hunk.is_none() {
        return Err(ApiError::conflict(
            "the selected hunk no longer matches the current workspace diff; refresh and retry",
        ));
    }

    let args: &[&str] = match (request.scope, request.action) {
        (WorkspaceDiffScope::Unstaged, WorkspaceDiffHunkAction::Stage) => &["apply", "--cached"],
        (WorkspaceDiffScope::Staged, WorkspaceDiffHunkAction::Unstage) => {
            &["apply", "--cached", "--reverse"]
        }
        (WorkspaceDiffScope::Unstaged, WorkspaceDiffHunkAction::Discard) => &["apply", "--reverse"],
        _ => {
            return Err(ApiError::bad_request(
                "invalid action for the selected diff scope",
            ))
        }
    };
    let mut check_args = args.to_vec();
    check_args.push("--check");
    run_git_with_input(&root, &check_args, &request.patch).await?;
    run_git_with_input(&root, args, &request.patch).await?;

    let diff = get_workspace_diff_inner(&root).await?;
    Ok(Json(WorkspaceDiffActionResponse {
        path: PathBuf::from(relative_path),
        diff,
    }))
}

async fn get_workspace_diff_inner(workspace_root: &FsPath) -> anyhow::Result<WorkspaceDiff> {
    let branch = run_git(workspace_root, ["symbolic-ref", "--short", "HEAD"])
        .await
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let remote_url = run_git(workspace_root, ["remote", "get-url", "origin"])
        .await
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let status_output = run_git(workspace_root, ["status", "--porcelain=v1"])
        .await
        .unwrap_or_else(|_| String::new());
    let staged_output = run_git(workspace_root, ["diff", "--cached", "--"])
        .await
        .unwrap_or_else(|_| String::new());
    let unstaged_output = run_git(workspace_root, ["diff", "--"])
        .await
        .unwrap_or_else(|_| String::new());
    let files = parse_git_status(&status_output);
    let (staged_diff, staged_truncated) = truncate_with_flag(&staged_output, 80_000);
    let (unstaged_diff, unstaged_truncated) = truncate_with_flag(&unstaged_output, 80_000);
    let mut hunks = parse_workspace_diff_hunks(&staged_diff, WorkspaceDiffScope::Staged);
    hunks.extend(parse_workspace_diff_hunks(
        &unstaged_diff,
        WorkspaceDiffScope::Unstaged,
    ));
    let diff = combine_workspace_diffs(&staged_diff, &unstaged_diff);
    Ok(WorkspaceDiff {
        command: "git diff --cached -- && git diff --".to_string(),
        branch,
        remote_url,
        files,
        diff,
        staged_diff,
        unstaged_diff,
        hunks,
        truncated: staged_truncated || unstaged_truncated,
        staged_truncated,
        unstaged_truncated,
    })
}

async fn get_context_status(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<ContextStatusResponse>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(context_status(&state, thread_id)?))
}

async fn compact_context(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<ContextCompactRequest>,
) -> Result<Json<ContextSummary>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let messages = state.store.list_messages(thread_id)?;
    let events = state.store.list_events(thread_id, None)?;
    let supplied_summary = request
        .summary
        .map(|summary| summary.trim().to_string())
        .filter(|summary| !summary.is_empty());
    let summary = if let Some(summary_text) = supplied_summary {
        let covered_through_seq = events.last().map(|event| event.seq).unwrap_or_default();
        let mut summary =
            ContextSummary::new(thread_id, covered_through_seq, messages.len(), summary_text);
        summary.token_estimate = Some(estimate_tokens(&summary.summary));
        summary.metadata = json!({
            "mode": "manual",
            "source": "context_compact_api",
            "coveredThroughSeq": covered_through_seq,
        });
        summary
    } else {
        generate_context_summary(&state, thread_id, &messages, &events, "context_compact_api")
            .await?
    };

    publish_payload(
        &state,
        thread_id,
        Some(Uuid::new_v4()),
        AgentEventPayload::ContextCompacted {
            summary: summary.clone(),
        },
    );
    Ok(Json(summary))
}

async fn get_sandbox(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<SandboxDescriptor>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    Ok(Json(SandboxDescriptor::local(
        thread_id,
        thread.workspace_root,
        &current_settings(&state).sandbox.to_local_sandbox_config(),
    )))
}

async fn run_browser_command(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<BrowserCommandRequest>,
) -> Result<Json<BrowserOutput>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let policy = BasicPolicyEngine::new(
        thread.workspace_root,
        current_settings(&state).permission_mode,
    );
    let session = BrowserSessionId::from_thread(thread_id);
    let timeout = request
        .timeout_ms
        .map(|milliseconds| Duration::from_millis(milliseconds.clamp(1, 120_000)));
    let result = match request.action.as_str() {
        "navigate" => {
            let url = browser_required(&request.url, "url")?;
            inspect_browser_url_policy(&state, thread_id, &policy, url)?;
            let mut command = BrowserNavigateRequest::new(url);
            if let Some(timeout) = timeout {
                command.wait = Some(BrowserWaitRequest {
                    condition: BrowserWaitCondition::DocumentComplete,
                    timeout: Some(timeout),
                    poll_interval: Duration::from_millis(100),
                });
            }
            state.browser.navigate(session, command).await
        }
        "snapshot" => state.browser.snapshot(session).await,
        "screenshot" => state.browser.screenshot(session).await,
        "click" => {
            inspect_browser_interaction_policy(&policy)?;
            state
                .browser
                .click(
                    session,
                    BrowserSelector::new(browser_required(&request.selector, "selector")?)
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                )
                .await
        }
        "type" => {
            inspect_browser_interaction_policy(&policy)?;
            state
                .browser
                .type_text(
                    session,
                    BrowserTypeRequest {
                        selector: BrowserSelector::new(browser_required(
                            &request.selector,
                            "selector",
                        )?)
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                        text: browser_required(&request.text, "text")?.to_string(),
                        clear_first: request.clear_first.unwrap_or(true),
                    },
                )
                .await
        }
        "wait" => {
            let condition = match request.condition.as_deref().unwrap_or("document_complete") {
                "document_complete" => BrowserWaitCondition::DocumentComplete,
                "selector" => BrowserWaitCondition::Selector(
                    BrowserSelector::new(browser_required(&request.selector, "selector")?)
                        .map_err(|error| ApiError::bad_request(error.to_string()))?,
                ),
                "text" => {
                    BrowserWaitCondition::Text(browser_required(&request.text, "text")?.to_string())
                }
                other => {
                    return Err(ApiError::bad_request(format!(
                        "unsupported browser wait condition: {other}"
                    )))
                }
            };
            state
                .browser
                .wait(
                    session,
                    BrowserWaitRequest {
                        condition,
                        timeout,
                        poll_interval: Duration::from_millis(100),
                    },
                )
                .await
        }
        "download" => {
            let url = browser_required(&request.url, "url")?;
            inspect_browser_url_policy(&state, thread_id, &policy, url)?;
            state
                .browser
                .download(
                    session,
                    BrowserDownloadRequest {
                        url: url.to_string(),
                        expected_filename: request.expected_filename,
                        timeout,
                    },
                )
                .await
        }
        "close" => {
            state
                .browser
                .close_session(session)
                .await
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            return Ok(Json(BrowserOutput {
                url: None,
                contents: Vec::new(),
                metadata: json!({ "action": "close" }),
            }));
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "unsupported browser action: {other}"
            )))
        }
    }
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(Json(result))
}

fn browser_required<'a>(value: &'a Option<String>, field: &str) -> Result<&'a str, ApiError> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request(format!("browser {field} is required")))
}

fn inspect_browser_url_policy(
    state: &AppState,
    thread_id: Uuid,
    policy: &BasicPolicyEngine,
    raw_url: &str,
) -> Result<(), ApiError> {
    let host = browser_domain_from_url(raw_url)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    match policy.inspect_network(&host) {
        PolicyDecision::Deny { reason } => return Err(ApiError::bad_request(reason)),
        // An explicit network approval policy and a first-visit domain policy both lead to the
        // same persisted, thread-scoped domain grant below.
        PolicyDecision::Allow | PolicyDecision::Ask { .. } => {}
    }

    if browser_domain_is_approved(state.store.as_ref(), thread_id, &host)? {
        return Ok(());
    }

    let action = browser_domain_approval_action(&host);
    let has_pending = state
        .store
        .list_approvals(thread_id, Some(ApprovalStatus::Pending))?
        .into_iter()
        .any(|approval| approval.action == action);
    if !has_pending {
        let approval_id = Uuid::new_v4();
        let reason =
            format!("Browser access to the new domain `{host}` requires approval for this thread.");
        state.store.insert_approval(Approval::pending(
            approval_id,
            thread_id,
            action.clone(),
            reason.clone(),
        ))?;
        publish_payload(
            state,
            thread_id,
            None,
            AgentEventPayload::ApprovalRequested {
                approval_id,
                action,
                reason,
            },
        );
    }

    Err(ApiError::conflict(format!(
        "approval required: Browser access to the new domain `{host}` is waiting for approval"
    )))
}

fn inspect_browser_interaction_policy(policy: &BasicPolicyEngine) -> Result<(), ApiError> {
    inspect_browser_policy_decision(policy.inspect_network("browser-interaction"))
}

fn inspect_browser_policy_decision(decision: PolicyDecision) -> Result<(), ApiError> {
    match decision {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } | PolicyDecision::Ask { reason } => {
            Err(ApiError::bad_request(reason))
        }
    }
}

async fn run_git_workflow(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(action): Json<GitWorkflowAction>,
) -> Result<Json<GitWorkflowResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let config = current_settings(&state).sandbox.to_local_sandbox_config();
    let environment =
        LocalExecutionEnvironment::with_sandbox_config(thread.workspace_root.clone(), config);
    let request = GitWorkflowRequest {
        repository: thread.workspace_root,
        action,
    };
    let result = execute_git_workflow(
        &environment,
        &request,
        ExecutionContext::with_timeout(Duration::from_secs(120)).with_resource_limits(
            ResourceLimit {
                max_output_bytes: Some(GIT_OUTPUT_BYTES_LIMIT),
                ..ResourceLimit::default()
            },
        ),
    )
    .await
    .map_err(|error| {
        let detail = error
            .failed_result()
            .map(|result| String::from_utf8_lossy(&result.stderr).trim().to_string())
            .filter(|detail| !detail.is_empty());
        ApiError::bad_request(detail.unwrap_or_else(|| error.to_string()))
    })?;
    Ok(Json(GitWorkflowResponse {
        action: result.action,
        stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        exit_code: result.exit_code,
        success: result.success,
        truncated: result.truncated,
    }))
}

async fn export_trajectory(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<TrajectoryExport>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let messages = state.store.list_messages(thread_id)?;
    let events = state.store.list_events(thread_id, None)?;
    let approvals = state.store.list_approvals(thread_id, None)?;
    let artifact_metas = state.store.list_artifacts(thread_id)?;
    let mut artifacts = Vec::new();
    for meta in &artifact_metas {
        if let Ok(Some(artifact)) = state.store.get_artifact(thread_id, meta.id) {
            artifacts.push(artifact);
        }
    }
    let workspace_diff = get_workspace_diff_inner(&thread.workspace_root).await.ok();
    Ok(Json(TrajectoryExport {
        exported_at: Utc::now(),
        thread,
        messages,
        events,
        approvals,
        artifacts,
        workspace_diff,
    }))
}

async fn list_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Json<Vec<AgentEvent>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    Ok(Json(state.store.list_events(thread_id, query.since)?))
}

async fn stream_events(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<EventQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let history = state.store.list_events(thread_id, query.since)?;
    let rx = state.events.subscribe(thread_id);
    let history_stream = stream::iter(history);
    let live_stream = BroadcastStream::new(rx).filter_map(|event| async move { event.ok() });
    let event_stream = history_stream.chain(live_stream).map(|agent_event| {
        let event_name = sse_event_name(agent_event.kind());
        let sse = Event::default()
            .event(event_name)
            .json_data(agent_event)
            .expect("agent event should serialize");
        Ok(sse)
    });

    Ok(Sse::new(event_stream).keep_alive(KeepAlive::default()))
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
    let session = spawn_pty_session(
        state.clone(),
        thread_id,
        thread.workspace_root,
        cwd,
        cols,
        rows,
    )?;
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

fn spawn_pty_session(
    state: AppState,
    thread_id: Uuid,
    workspace_root: PathBuf,
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
    let sandbox_config = current_settings(&state).sandbox.to_local_sandbox_config();
    let command_plan =
        build_local_sandbox_command(&shell, &shell_args, &cwd, &workspace_root, &sandbox_config)?;
    let mut command = CommandBuilder::new(&command_plan.program);
    command.cwd(shell_native_path(&cwd));
    for key in SENSITIVE_CHILD_ENV_KEYS {
        command.env_remove(key);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for (key, value) in &command_plan.env {
        command.env(key, value);
    }
    for arg in &command_plan.args {
        command.arg(arg);
    }

    let mut child = pair.slave.spawn_command(command)?;
    let process_id = child.process_id();
    let killer = child.clone_killer();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let session_id = Uuid::new_v4();
    let cwd_display = cwd.to_string_lossy().to_string();
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
        (
            std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string()),
            if std::env::var("COMSPEC").is_ok() {
                Vec::new()
            } else {
                vec!["-NoLogo".to_string(), "-NoProfile".to_string()]
            },
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

    let exec_request = ExecRequest::shell(command.clone()).cwd(cwd.clone());
    let sandbox_config = current_settings(&state).sandbox.to_local_sandbox_config();
    let command_plan = build_local_sandbox_command(
        &exec_request.program,
        &exec_request.args,
        &cwd,
        &thread.workspace_root,
        &sandbox_config,
    )?;
    let mut process = Command::new(&command_plan.program);
    for key in SENSITIVE_CHILD_ENV_KEYS {
        process.env_remove(key);
    }
    process
        .args(&command_plan.args)
        .envs(command_plan.env)
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
        let sse = Event::default()
            .event(terminal_event.kind.sse_event_name())
            .json_data(terminal_event)
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

async fn run_terminal_command(
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

async fn list_mcp_servers(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpServerView>>, ApiError> {
    let servers = state.store.list_mcp_servers()?;
    Ok(Json(
        servers
            .into_iter()
            .map(|server| McpServerView {
                status: McpServerStatus::from_config(&server),
                server,
            })
            .collect(),
    ))
}

async fn create_mcp_server(
    State(state): State<AppState>,
    Json(request): Json<McpServerRequest>,
) -> Result<Json<McpServerView>, ApiError> {
    let server = request.into_config()?;
    let server = state.store.insert_mcp_server(server)?;
    Ok(Json(McpServerView {
        status: McpServerStatus::from_config(&server),
        server,
    }))
}

async fn update_mcp_server(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Json(request): Json<McpServerPatchRequest>,
) -> Result<Json<McpServerView>, ApiError> {
    let mut server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    request.apply(&mut server)?;
    server.refresh_updated_at();
    let server = state
        .store
        .update_mcp_server(server)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    Ok(Json(McpServerView {
        status: McpServerStatus::from_config(&server),
        server,
    }))
}

async fn delete_mcp_server(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<DeleteResponse>, ApiError> {
    state.mcp_host.stop_server(server_id).await.ok();
    let deleted = state.store.delete_mcp_server(server_id)?;
    if !deleted {
        return Err(ApiError::not_found(format!(
            "MCP server not found: {server_id}"
        )));
    }
    Ok(Json(DeleteResponse { deleted }))
}

async fn restart_mcp_server(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<McpServerStatus>, ApiError> {
    let server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    let status = state.mcp_host.restart_server(server).await?;
    Ok(Json(status))
}

async fn list_mcp_tools(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
) -> Result<Json<Vec<McpToolDescriptor>>, ApiError> {
    state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    Ok(Json(state.mcp_host.cached_tools(server_id).await))
}

async fn call_mcp_tool(
    State(state): State<AppState>,
    Path(server_id): Path<Uuid>,
    Json(request): Json<McpToolCallRequest>,
) -> Result<Json<McpCallResult>, ApiError> {
    let server = state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    let thread_id = request.thread_id;
    let thread = state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))?;
    let enabled_for_thread = state
        .store
        .list_thread_mcp_servers(thread_id)?
        .into_iter()
        .any(|binding| binding.server_id == server_id && binding.enabled);
    if !server.enabled || !enabled_for_thread {
        return Err(ApiError::bad_request(
            "MCP server is not enabled for this thread",
        ));
    }

    let tools = state.mcp_host.cached_tools(server_id).await;
    let descriptor = tools
        .iter()
        .find(|t| t.tool_name == request.tool_name)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "tool not found on server {}: {}",
                server_id, request.tool_name
            ))
        })?;

    let settings = current_settings(&state);
    let policy = Arc::new(BasicPolicyEngine::new(
        thread.workspace_root,
        settings.permission_mode,
    ));
    let permission = ToolPermissionDescriptor::from(descriptor);
    match policy.inspect_mcp_tool_call(&permission) {
        PolicyDecision::Allow => {}
        PolicyDecision::Deny { reason } => return Err(ApiError::bad_request(reason)),
        PolicyDecision::Ask { reason } => return Err(ApiError::bad_request(reason)),
    }

    let call = ToolCall::new(&descriptor.public_name, request.arguments.clone());
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::ToolCallStarted { call: call.clone() },
    );

    let result = match state
        .mcp_host
        .call_tool(&descriptor.public_name, request.arguments)
        .await
    {
        Ok(result) => result,
        Err(err) => {
            let tool_result = ToolResult {
                call_id: call.id,
                output: err.to_string(),
                content: vec![ModelContentPart::text(err.to_string())],
                metadata: json!({
                    "success": false,
                    "error": err.to_string(),
                    "publicName": descriptor.public_name,
                    "toolName": descriptor.tool_name,
                    "serverId": descriptor.server_id,
                }),
            };
            publish_payload(
                &state,
                thread_id,
                None,
                AgentEventPayload::ToolCallFinished {
                    result: tool_result,
                },
            );
            return Err(ApiError::from(err));
        }
    };

    let tool_result = ToolResult {
        call_id: call.id,
        output: result.output.clone(),
        content: result
            .structured_content
            .clone()
            .map(ModelContentPart::json)
            .into_iter()
            .collect(),
        metadata: json!({
            "isError": result.is_error,
            "publicName": descriptor.public_name,
            "toolName": descriptor.tool_name,
            "serverId": descriptor.server_id,
        }),
    };
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::ToolCallFinished {
            result: tool_result,
        },
    );

    Ok(Json(result))
}

async fn list_thread_mcp_servers(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<ThreadMcpServerView>>, ApiError> {
    ensure_thread(&state, thread_id)?;
    let bindings = state.store.list_thread_mcp_servers(thread_id)?;
    let bindings_by_server = bindings
        .into_iter()
        .map(|binding| (binding.server_id, binding))
        .collect::<HashMap<_, _>>();
    let servers = state.store.list_mcp_servers()?;
    Ok(Json(
        servers
            .into_iter()
            .map(|server| {
                let binding = bindings_by_server.get(&server.server_id).cloned();
                let enabled = server.enabled && binding.as_ref().is_some_and(|item| item.enabled);
                ThreadMcpServerView {
                    enabled,
                    binding,
                    server,
                }
            })
            .collect(),
    ))
}

async fn set_thread_mcp_server(
    State(state): State<AppState>,
    Path((thread_id, server_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<ThreadMcpServerRequest>,
) -> Result<Json<ThreadMcpServer>, ApiError> {
    ensure_thread(&state, thread_id)?;
    state
        .store
        .get_mcp_server(server_id)?
        .ok_or_else(|| ApiError::not_found(format!("MCP server not found: {server_id}")))?;
    Ok(Json(state.store.set_thread_mcp_server(
        thread_id,
        server_id,
        request.enabled,
    )?))
}

fn sse_event_name(kind: &str) -> &str {
    if kind == "error" {
        "agent_error"
    } else {
        kind
    }
}

fn ensure_thread(state: &AppState, thread_id: Uuid) -> Result<opentopia_core::Thread, ApiError> {
    state
        .store
        .get_thread(thread_id)?
        .ok_or_else(|| ApiError::not_found(format!("thread not found: {thread_id}")))
}

async fn sync_thread_mcp_tools(store: &SqliteSessionStore, thread_id: Uuid, agent: &mut AgentCore) {
    let enabled_servers = match store.list_mcp_servers() {
        Ok(servers) => servers
            .into_iter()
            .filter(|server| server.enabled)
            .map(|server| server.server_id)
            .collect::<HashSet<_>>(),
        Err(err) => {
            error!(?err, %thread_id, "failed to load MCP server configuration");
            return;
        }
    };
    let server_ids = match store.list_thread_mcp_servers(thread_id) {
        Ok(bindings) => bindings
            .into_iter()
            .filter(|binding| binding.enabled && enabled_servers.contains(&binding.server_id))
            .map(|binding| binding.server_id)
            .collect::<Vec<_>>(),
        Err(err) => {
            error!(?err, %thread_id, "failed to load thread MCP bindings");
            return;
        }
    };
    agent.sync_mcp_tools_for_servers(&server_ids).await;
}

fn publish_payload(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Option<Uuid>,
    payload: AgentEventPayload,
) {
    let event = AgentEvent::new(thread_id, turn_id, 0, payload);
    match state.store.append_event(event) {
        Ok(event) => state.events.publish(event),
        Err(err) => error!(?err, "failed to persist event"),
    }
}

async fn run_new_agent_turn(
    state: AppState,
    thread: opentopia_core::Thread,
    user_message: Message,
    content: String,
    user_content: Vec<ModelContentPart>,
    turn: TurnHandle,
) {
    let thread_id = thread.id;
    let turn_id = turn.turn_id;
    let prepared_result = tokio::select! {
        _ = turn.cancel.cancelled() => {
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::TurnCancelled {
                    reason: "Cancelled by user.".to_string(),
                },
            );
            state.turns.finish(thread_id, turn_id);
            return;
        }
        prepared = prepare_turn_context(&state, thread_id, user_message.id) => prepared,
    };
    let prepared = match prepared_result {
        Ok(prepared) => prepared,
        Err(err) => {
            publish_payload(
                &state,
                thread_id,
                Some(turn_id),
                AgentEventPayload::Error {
                    message: err.message,
                },
            );
            state.turns.finish(thread_id, turn_id);
            return;
        }
    };
    let settings = current_settings(&state);
    let input = AgentTurnInput {
        thread_id,
        user_message_id: user_message.id,
        workspace_root: thread.workspace_root,
        content,
        user_content,
        context_summary: prepared.summary,
        conversation: prepared.conversation,
        permission_mode: settings.permission_mode,
        context_budget: Some(prepared.budget),
        store: Some(state.store.clone()),
        cancellation: Some(turn.cancel.clone()),
    };

    let mut agent = state.agent.read().expect("agent lock poisoned").clone();
    agent.set_mcp_host(state.mcp_host.clone());
    agent.set_subagent_context(turn_id, 0);
    sync_thread_mcp_tools(&state.store, thread_id, &mut agent).await;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let future = agent.run_turn_detailed_streaming(input, Some(sender));
    tokio::pin!(future);
    let mut deferred_approval_events = Vec::new();

    let result = loop {
        tokio::select! {
            biased;
            _ = turn.cancel.cancelled() => {
                publish_payload(
                    &state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::TurnCancelled {
                        reason: "Cancelled by user.".to_string(),
                    },
                );
                let _ = timeout(Duration::from_secs(2), &mut future).await;
                while let Ok(payload) = receiver.try_recv() {
                    persist_and_publish_payload(&state, thread_id, turn_id, payload);
                }
                state.turns.finish(thread_id, turn_id);
                return;
            }
            result = &mut future => break result,
            payload = receiver.recv() => {
                if let Some(payload) = payload {
                    if is_approval_boundary(&payload) {
                        deferred_approval_events.push(payload);
                    } else {
                        persist_and_publish_payload(&state, thread_id, turn_id, payload);
                    }
                }
            }
        }
    };
    while let Ok(payload) = receiver.try_recv() {
        if is_approval_boundary(&payload) {
            deferred_approval_events.push(payload);
        } else {
            persist_and_publish_payload(&state, thread_id, turn_id, payload);
        }
    }
    persist_deferred_approval_records(&state, thread_id, &deferred_approval_events);
    persist_suspended_continuation(&state, thread_id, turn_id, &result);
    for payload in deferred_approval_events {
        publish_payload(&state, thread_id, Some(turn_id), payload);
    }
    finish_agent_result(&state, thread_id, turn_id, result, None);
    state.turns.finish(thread_id, turn_id);
}

async fn run_resumed_agent_turn(
    state: AppState,
    approval_id: Uuid,
    continuation: AgentContinuation,
    approved: bool,
    turn: TurnHandle,
) {
    let thread_id = continuation.thread_id;
    let turn_id = turn.turn_id;
    let mut agent = state.agent.read().expect("agent lock poisoned").clone();
    agent.set_mcp_host(state.mcp_host.clone());
    agent.set_subagent_context(turn_id, 0);
    sync_thread_mcp_tools(&state.store, thread_id, &mut agent).await;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let future = agent.resume_turn_streaming(
        continuation,
        approved,
        Some(state.store.clone()),
        Some(turn.cancel.clone()),
        Some(sender),
    );
    tokio::pin!(future);
    let mut deferred_approval_events = Vec::new();

    let result = loop {
        tokio::select! {
            biased;
            _ = turn.cancel.cancelled() => {
                publish_payload(
                    &state,
                    thread_id,
                    Some(turn_id),
                    AgentEventPayload::TurnCancelled {
                        reason: "Cancelled by user.".to_string(),
                    },
                );
                let _ = timeout(Duration::from_secs(2), &mut future).await;
                while let Ok(payload) = receiver.try_recv() {
                    persist_and_publish_payload(&state, thread_id, turn_id, payload);
                }
                state.turns.finish(thread_id, turn_id);
                return;
            }
            result = &mut future => break result,
            payload = receiver.recv() => {
                if let Some(payload) = payload {
                    if is_approval_boundary(&payload) {
                        deferred_approval_events.push(payload);
                    } else {
                        persist_and_publish_payload(&state, thread_id, turn_id, payload);
                    }
                }
            }
        }
    };
    while let Ok(payload) = receiver.try_recv() {
        if is_approval_boundary(&payload) {
            deferred_approval_events.push(payload);
        } else {
            persist_and_publish_payload(&state, thread_id, turn_id, payload);
        }
    }
    persist_deferred_approval_records(&state, thread_id, &deferred_approval_events);
    persist_suspended_continuation(&state, thread_id, turn_id, &result);
    for payload in deferred_approval_events {
        publish_payload(&state, thread_id, Some(turn_id), payload);
    }
    finish_agent_result(&state, thread_id, turn_id, result, Some(approval_id));
    state.turns.finish(thread_id, turn_id);
}

fn finish_agent_result(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    result: anyhow::Result<opentopia_core::AgentTurnResult>,
    resolved_approval_id: Option<Uuid>,
) {
    match result {
        Ok(_) => {}
        Err(err) => publish_payload(
            state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: err.to_string(),
            },
        ),
    }
    if let Some(approval_id) = resolved_approval_id {
        if let Err(err) = state
            .store
            .delete_approval_continuation(approval_id, thread_id)
        {
            error!(?err, %approval_id, "failed to remove resolved continuation");
        }
    }
}

fn persist_suspended_continuation(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    result: &anyhow::Result<opentopia_core::AgentTurnResult>,
) {
    let Ok(result) = result else {
        return;
    };
    let AgentTurnOutcome::Suspended {
        approval_id,
        continuation,
    } = &result.outcome
    else {
        return;
    };
    let persist_result = serde_json::to_value(continuation)
        .map_err(anyhow::Error::from)
        .and_then(|value| {
            state
                .store
                .put_approval_continuation(*approval_id, thread_id, value)
        });
    if let Err(err) = persist_result {
        error!(?err, %approval_id, "failed to persist approval continuation");
        publish_payload(
            state,
            thread_id,
            Some(turn_id),
            AgentEventPayload::Error {
                message: format!("failed to persist approval continuation: {err}"),
            },
        );
    }
}

fn persist_and_publish_payload(
    state: &AppState,
    thread_id: Uuid,
    turn_id: Uuid,
    payload: AgentEventPayload,
) {
    if let AgentEventPayload::AssistantMessage { message } = &payload {
        if let Err(err) = state.store.append_message(message.clone()) {
            error!(?err, "failed to persist assistant message");
        }
    }
    if let AgentEventPayload::ApprovalRequested {
        approval_id,
        action,
        reason,
    } = &payload
    {
        let approval = Approval::pending(*approval_id, thread_id, action.clone(), reason.clone());
        if let Err(err) = state.store.insert_approval(approval) {
            error!(?err, %approval_id, "failed to persist approval request");
        }
    }
    publish_payload(state, thread_id, Some(turn_id), payload);
}

fn is_approval_boundary(payload: &AgentEventPayload) -> bool {
    matches!(
        payload,
        AgentEventPayload::ApprovalRequested { .. } | AgentEventPayload::TurnSuspended { .. }
    )
}

fn persist_deferred_approval_records(
    state: &AppState,
    thread_id: Uuid,
    payloads: &[AgentEventPayload],
) {
    for payload in payloads {
        let AgentEventPayload::ApprovalRequested {
            approval_id,
            action,
            reason,
        } = payload
        else {
            continue;
        };
        let approval = Approval::pending(*approval_id, thread_id, action.clone(), reason.clone());
        if let Err(err) = state.store.insert_approval(approval) {
            error!(?err, %approval_id, "failed to persist approval request");
        }
    }
}

struct PreparedTurnContext {
    summary: Option<String>,
    conversation: Vec<ModelConversationMessage>,
    budget: AgentContextBudget,
}

async fn prepare_turn_context(
    state: &AppState,
    thread_id: Uuid,
    current_message_id: Uuid,
) -> Result<PreparedTurnContext, ApiError> {
    let messages = state.store.list_messages(thread_id)?;
    let events = state.store.list_events(thread_id, None)?;
    let mut summary = latest_context_summary_event(&events);
    let prior_messages = messages
        .iter()
        .filter(|message| message.id != current_message_id)
        .cloned()
        .collect::<Vec<_>>();
    let covered = summary
        .as_ref()
        .map(|summary| summary.message_count)
        .unwrap_or_default()
        .min(prior_messages.len());
    let unsummarized_tokens = prior_messages
        .iter()
        .skip(covered)
        .map(message_token_estimate)
        .sum::<usize>();
    let summary_tokens = summary
        .as_ref()
        .map(|summary| estimate_tokens(&summary.summary))
        .unwrap_or_default();
    let context_window = context_window_tokens();
    let usage_percent = summary_tokens
        .saturating_add(unsummarized_tokens)
        .saturating_mul(100)
        / context_window.max(1);
    if prior_messages.len().saturating_sub(covered) >= 6
        && usage_percent >= context_compact_threshold_percent()
        && current_settings(state).active_provider().kind != ProviderKind::Mock
    {
        match generate_context_summary(
            state,
            thread_id,
            &prior_messages,
            &events,
            "automatic_threshold",
        )
        .await
        {
            Ok(compacted) => {
                publish_payload(
                    state,
                    thread_id,
                    None,
                    AgentEventPayload::ContextCompacted {
                        summary: compacted.clone(),
                    },
                );
                summary = Some(compacted);
            }
            Err(err) => {
                error!(message = %err.message, "automatic context compaction failed");
            }
        }
    }
    let covered_messages = summary
        .as_ref()
        .map(|summary| summary.message_count)
        .unwrap_or_default()
        .min(messages.len());
    let history_limit = context_window.saturating_mul(65) / 100;
    let mut used = summary
        .as_ref()
        .map(|summary| estimate_tokens(&summary.summary))
        .unwrap_or_default();
    let mut conversation = Vec::new();
    for message in messages.iter().skip(covered_messages).rev() {
        if message.id == current_message_id {
            continue;
        }
        let Some(message) = model_conversation_message(message) else {
            continue;
        };
        let tokens = estimate_tokens(&message.content).saturating_add(8);
        if !conversation.is_empty() && used.saturating_add(tokens) > history_limit {
            break;
        }
        used = used.saturating_add(tokens);
        conversation.push(message);
    }
    conversation.reverse();

    let mut budget = AgentContextBudget::new(context_window);
    budget.record_tokens(used);
    Ok(PreparedTurnContext {
        summary: summary.map(|summary| summary.summary),
        conversation,
        budget,
    })
}

fn model_conversation_message(message: &Message) -> Option<ModelConversationMessage> {
    let role = match message.role {
        MessageRole::User => ModelConversationRole::User,
        MessageRole::Assistant => ModelConversationRole::Assistant,
        MessageRole::System => ModelConversationRole::System,
        MessageRole::Tool => return None,
    };
    let content = message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let content_parts = message
        .parts
        .iter()
        .flat_map(message_model_content_parts)
        .collect::<Vec<_>>();
    (!content.trim().is_empty() || !content_parts.is_empty()).then_some(ModelConversationMessage {
        role,
        content,
        content_parts,
    })
}

fn message_model_content_parts(part: &MessagePart) -> Vec<ModelContentPart> {
    match part {
        MessagePart::ToolResult { result } => result.content_or_legacy_text(),
        MessagePart::SourceRef { source } => vec![ModelContentPart::resource(
            source.path.to_string_lossy(),
            Some(source.content_type.clone()),
            Some(source.name.clone()),
        )],
        _ => Vec::new(),
    }
}

fn estimate_tokens(text: &str) -> usize {
    (text.len() + 3) / 4
}

fn message_token_estimate(message: &Message) -> usize {
    message
        .parts
        .iter()
        .map(|part| match part {
            MessagePart::Text { text } => estimate_tokens(text),
            MessagePart::ToolResult { result } => estimate_tokens(&result.output),
            MessagePart::ToolCall { .. } => 64,
            _ => 16,
        })
        .sum::<usize>()
        .saturating_add(12)
}

fn context_window_tokens() -> usize {
    std::env::var("OPENTOPIA_CONTEXT_WINDOW_TOKENS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value >= 4_096)
        .unwrap_or(128_000)
}

fn context_compact_threshold_percent() -> usize {
    std::env::var("OPENTOPIA_CONTEXT_COMPACT_THRESHOLD_PERCENT")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|value: usize| value.clamp(50, 95))
        .unwrap_or(80)
}

fn current_settings(state: &AppState) -> AppSettings {
    state
        .settings
        .read()
        .expect("settings lock poisoned")
        .clone()
}

fn context_status(state: &AppState, thread_id: Uuid) -> Result<ContextStatusResponse, ApiError> {
    let budget = state.store.get_context_budget(thread_id)?;
    let events = state.store.list_events(thread_id, None)?;
    let latest_summary = latest_context_summary_event(&events);
    Ok(ContextStatusResponse {
        budget,
        latest_summary,
    })
}

fn latest_context_summary_event(events: &[AgentEvent]) -> Option<ContextSummary> {
    events.iter().rev().find_map(|event| {
        if let AgentEventPayload::ContextCompacted { summary } = &event.payload {
            Some(summary.clone())
        } else {
            None
        }
    })
}

async fn generate_context_summary(
    state: &AppState,
    thread_id: Uuid,
    messages: &[Message],
    events: &[AgentEvent],
    source: &str,
) -> Result<ContextSummary, ApiError> {
    let settings = current_settings(state);
    let active = settings.active_provider().clone();
    if active.kind == ProviderKind::Mock {
        return Err(ApiError::bad_request(
            "real context summarization requires an OpenAI-compatible provider",
        ));
    }
    let provider = OpenAiCompatibleProvider::from_settings(&active).ok_or_else(|| {
        ApiError::bad_request(format!(
            "provider '{}' has no configured API key",
            active.id
        ))
    })?;
    let snapshot = build_context_snapshot(messages, events);
    let response = timeout(
        Duration::from_secs(90),
        provider.complete(ModelRequest {
            system_prompt: context_summary_system_prompt().to_string(),
            conversation: Vec::new(),
            user_message: snapshot,
            user_content: Vec::new(),
            tool_candidates: Vec::new(),
            previous_tool_calls: Vec::new(),
            tool_results: Vec::new(),
        }),
    )
    .await
    .map_err(|_| ApiError::gateway_timeout("context summarization timed out"))?
    .map_err(|err| ApiError::bad_gateway(format!("context summarization failed: {err}")))?;
    if response.text.trim().is_empty() {
        return Err(ApiError::bad_gateway(
            "context summarization provider returned empty text",
        ));
    }

    let covered_through_seq = events.last().map(|event| event.seq).unwrap_or_default();
    let mut summary = ContextSummary::new(
        thread_id,
        covered_through_seq,
        messages.len(),
        response.text.trim(),
    );
    summary.token_estimate = Some(estimate_tokens(&summary.summary));
    summary.metadata = json!({
        "mode": "llm",
        "source": source,
        "providerId": active.id,
        "model": active.model,
        "coveredThroughSeq": covered_through_seq,
    });
    Ok(summary)
}

fn context_summary_system_prompt() -> &'static str {
    "You compress an AI coding-agent session into durable working memory. Return only the summary, using short sections: Goal, Decisions, Changes, Commands and validation, Open issues, Next steps. Preserve exact file paths, commands, errors, identifiers, user constraints, and unresolved risks. Omit greetings, repetition, transient progress narration, and secrets. Never invent completed work."
}

fn build_context_snapshot(messages: &[Message], events: &[AgentEvent]) -> String {
    const MAX_SNAPSHOT_CHARS: usize = 96_000;
    let mut sections = Vec::new();
    let mut used = 0usize;

    for message in messages.iter().rev() {
        let rendered = render_message_for_summary(message);
        let chars = rendered.chars().count();
        if used + chars > MAX_SNAPSHOT_CHARS {
            break;
        }
        used += chars;
        sections.push(rendered);
    }
    sections.reverse();

    let mut event_lines = Vec::new();
    for event in events.iter().rev().take(160).rev() {
        let rendered = match &event.payload {
            AgentEventPayload::ModelDelta { .. }
            | AgentEventPayload::AssistantMessage { .. }
            | AgentEventPayload::TurnStarted { .. } => continue,
            payload => serde_json::to_string(payload)
                .unwrap_or_else(|_| format!("{{\"type\":\"{}\"}}", payload.kind())),
        };
        event_lines.push(format!(
            "seq={} {}",
            event.seq,
            truncate_chars(&rendered, 2_000)
        ));
    }

    format!(
        "Summarize this session snapshot. Messages are ordered oldest to newest.\n\nMESSAGES\n{}\n\nIMPORTANT EVENTS\n{}",
        sections.join("\n\n"),
        event_lines.join("\n")
    )
}

fn render_message_for_summary(message: &Message) -> String {
    let parts = message
        .parts
        .iter()
        .map(|part| match part {
            MessagePart::Text { text } => truncate_chars(text, 12_000),
            MessagePart::ToolCall { call } => format!(
                "tool_call {} {}",
                call.name,
                truncate_chars(&call.input.to_string(), 4_000)
            ),
            MessagePart::ToolResult { result } => format!(
                "tool_result {} {}",
                result.call_id,
                truncate_chars(&result.output, 12_000)
            ),
            MessagePart::FileRef { path } => format!("file_ref {}", path.display()),
            MessagePart::SourceRef { source } => format!(
                "source_ref {} {} {} bytes{}",
                source.name,
                source.path.display(),
                source.bytes,
                if source.truncated { " truncated" } else { "" }
            ),
            MessagePart::SkillRef { skill } => format!(
                "skill_ref {} {}{}",
                skill.name,
                skill.path.display(),
                if skill.truncated { " truncated" } else { "" }
            ),
            MessagePart::Error { message } => format!("error {}", truncate_chars(message, 4_000)),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[{} {}]\n{}",
        message.role.as_str(),
        message.created_at.to_rfc3339(),
        parts
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        let mut output = value.chars().take(max_chars).collect::<String>();
        output.push_str("\n[truncated]");
        output
    }
}

fn canonical_workspace_root(workspace_root: &FsPath) -> PathBuf {
    workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf())
}

fn resolve_workspace_path(root: &FsPath, requested: Option<&str>) -> Result<PathBuf, ApiError> {
    let requested = requested.unwrap_or(".").trim();
    let requested = if requested.is_empty() { "." } else { requested };
    let raw = PathBuf::from(requested);
    if raw
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ApiError::bad_request("workspace path cannot contain .."));
    }
    let candidate = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    };
    let resolved = candidate.canonicalize().map_err(|_| {
        ApiError::not_found(format!("workspace path not found: {}", candidate.display()))
    })?;
    if !resolved.starts_with(root) {
        return Err(ApiError::bad_request(format!(
            "path is outside workspace: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
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

fn list_workspace_entries(root: &FsPath, path: &FsPath) -> Result<Vec<WorkspaceEntry>, ApiError> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| ApiError::not_found(format!("path not found: {}", path.display())))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request(format!(
            "path is not a directory: {}",
            path.display()
        )));
    }

    let mut entries = std::fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let entry_path = entry.path();
            let metadata = entry.metadata()?;
            let file_type = entry.file_type()?;
            let kind = if file_type.is_symlink() {
                WorkspaceEntryKind::Symlink
            } else if metadata.is_dir() {
                WorkspaceEntryKind::Directory
            } else if metadata.is_file() {
                WorkspaceEntryKind::File
            } else {
                WorkspaceEntryKind::Other
            };
            Ok(WorkspaceEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                path: relative_workspace_path(root, &entry_path),
                kind,
                size: metadata.is_file().then_some(metadata.len()),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            })
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    entries.sort_by(|left, right| {
        let left_dir = left.kind == WorkspaceEntryKind::Directory;
        let right_dir = right.kind == WorkspaceEntryKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

fn relative_workspace_path(root: &FsPath, path: &FsPath) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn validate_relative_git_path(path: &str) -> Result<String, ApiError> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(ApiError::bad_request("path cannot be empty"));
    }
    if normalized.contains(" -> ") {
        return Err(ApiError::bad_request(
            "renamed paths must be reverted manually for now",
        ));
    }
    let path_buf = PathBuf::from(&normalized);
    if path_buf.is_absolute()
        || path_buf.components().any(|component| {
            matches!(
                component,
                std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
                    | std::path::Component::ParentDir
            )
        })
    {
        return Err(ApiError::bad_request(
            "path must be a relative workspace path without ..",
        ));
    }
    Ok(normalized)
}

async fn run_git<const N: usize>(
    workspace_root: &FsPath,
    args: [&str; N],
) -> Result<String, ApiError> {
    let output = timeout(
        Duration::from_secs(20),
        Command::new("git")
            .args(args)
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| ApiError::bad_request("git command timed out"))??;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_git_with_input(
    workspace_root: &FsPath,
    args: &[&str],
    input: &str,
) -> Result<String, ApiError> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await?;
    }
    let output = timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| ApiError::bad_request("git command timed out"))??;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_git_status(output: &str) -> Vec<ChangedFile> {
    output
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let status_code = &line[..2];
            let mut path = line[3..].trim();
            if path.is_empty() {
                return None;
            }
            let mut original_path = None;
            let is_renamed = status_code.contains('R') || status_code.contains('C');
            if is_renamed {
                if let Some((original, renamed)) = path.split_once(" -> ") {
                    original_path = Some(PathBuf::from(original));
                    path = renamed;
                }
            }
            let is_untracked = status_code == "??";
            let staged_status = if is_untracked {
                String::new()
            } else {
                git_status_name(status_code.chars().next().unwrap_or(' '))
            };
            let unstaged_status = if is_untracked {
                "untracked".to_string()
            } else {
                git_status_name(status_code.chars().nth(1).unwrap_or(' '))
            };
            let status = if is_untracked {
                "??".to_string()
            } else {
                status_code.trim().to_string()
            };
            Some(ChangedFile {
                path: PathBuf::from(path),
                status,
                staged_status,
                unstaged_status,
                original_path,
                is_untracked,
                is_renamed,
            })
        })
        .collect()
}

fn git_status_name(status: char) -> String {
    match status {
        'M' => "modified",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'U' => "unmerged",
        '?' => "untracked",
        '!' => "ignored",
        _ => "",
    }
    .to_string()
}

fn combine_workspace_diffs(staged_diff: &str, unstaged_diff: &str) -> String {
    match (
        staged_diff.trim().is_empty(),
        unstaged_diff.trim().is_empty(),
    ) {
        (true, true) => String::new(),
        (false, true) => staged_diff.to_string(),
        (true, false) => unstaged_diff.to_string(),
        (false, false) => format!(
            "# staged: git diff --cached --\n{}\n\n# unstaged: git diff --\n{}",
            staged_diff.trim_end(),
            unstaged_diff.trim_start()
        ),
    }
}

fn parse_workspace_diff_hunks(diff: &str, scope: WorkspaceDiffScope) -> Vec<WorkspaceDiffHunk> {
    let mut hunks = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_hunk: Option<WorkspaceDiffHunk> = None;
    let mut current_file_header = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            push_diff_hunk(&mut hunks, &mut current_hunk);
            current_path = parse_diff_git_path(line);
            current_file_header.clear();
            current_file_header.push(line.to_string());
            continue;
        }

        if let Some(path) = line.strip_prefix("--- ") {
            if current_path.is_none() {
                current_path = parse_diff_marker_path(path);
            }
            current_file_header.push(line.to_string());
            continue;
        }

        if let Some(path) = line.strip_prefix("+++ ") {
            if let Some(parsed_path) = parse_diff_marker_path(path) {
                current_path = Some(parsed_path);
            }
            current_file_header.push(line.to_string());
            continue;
        }

        if line.starts_with("@@ ") {
            push_diff_hunk(&mut hunks, &mut current_hunk);
            if let Some(path) = current_path.clone() {
                let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(line);
                current_hunk = Some(WorkspaceDiffHunk {
                    path,
                    scope,
                    header: line.to_string(),
                    lines: Vec::new(),
                    raw: line.to_string(),
                    patch: format!("{}\n{}\n", current_file_header.join("\n"), line),
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                });
            }
            continue;
        }

        if let Some(hunk) = &mut current_hunk {
            hunk.lines.push(line.to_string());
            hunk.raw.push('\n');
            hunk.raw.push_str(line);
            hunk.patch.push_str(line);
            hunk.patch.push('\n');
        } else if !current_file_header.is_empty() {
            current_file_header.push(line.to_string());
        }
    }

    push_diff_hunk(&mut hunks, &mut current_hunk);
    hunks
}

fn push_diff_hunk(
    hunks: &mut Vec<WorkspaceDiffHunk>,
    current_hunk: &mut Option<WorkspaceDiffHunk>,
) {
    if let Some(hunk) = current_hunk.take() {
        hunks.push(hunk);
    }
}

fn parse_hunk_header(header: &str) -> (Option<u32>, Option<u32>, Option<u32>, Option<u32>) {
    let Some(range_end) = header[3..].find("@@").map(|index| index + 3) else {
        return (None, None, None, None);
    };
    let mut ranges = header[3..range_end].split_whitespace();
    let (old_start, old_lines) = ranges
        .next()
        .and_then(|range| parse_hunk_range(range, '-'))
        .unwrap_or((None, None));
    let (new_start, new_lines) = ranges
        .next()
        .and_then(|range| parse_hunk_range(range, '+'))
        .unwrap_or((None, None));
    (old_start, old_lines, new_start, new_lines)
}

fn parse_hunk_range(range: &str, prefix: char) -> Option<(Option<u32>, Option<u32>)> {
    let range = range.strip_prefix(prefix)?;
    let (start, lines) = range
        .split_once(',')
        .map(|(start, lines)| (start, lines))
        .unwrap_or((range, "1"));
    Some((start.parse().ok(), lines.parse().ok()))
}

fn parse_diff_git_path(line: &str) -> Option<PathBuf> {
    line.rsplit_once(" b/")
        .map(|(_, path)| PathBuf::from(unquote_git_path(path.trim())))
}

fn parse_diff_marker_path(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path == "/dev/null" {
        return None;
    }
    path.strip_prefix("b/")
        .or_else(|| path.strip_prefix("a/"))
        .map(|path| PathBuf::from(unquote_git_path(path.trim())))
}

fn unquote_git_path(path: &str) -> String {
    path.trim_matches('"').replace("\\\"", "\"")
}

fn normalized_path_string(path: &FsPath) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn truncate_with_flag(value: &str, max_bytes: usize) -> (String, bool) {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    ok: bool,
    service: &'static str,
    #[serde(rename = "apiVersion")]
    api_version: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GitWorkflowResponse {
    action: opentopia_core::GitWorkflowActionKind,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    success: bool,
    truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatchRequest {
    providers: Option<Vec<ProviderSettings>>,
    active_provider_id: Option<String>,
    provider_kind: Option<ProviderKind>,
    base_url: Option<String>,
    model: Option<String>,
    api_key_source: Option<String>,
    permission_mode: Option<PermissionMode>,
    default_workspace_root: Option<PathBuf>,
    clear_default_workspace_root: Option<bool>,
    sandbox: Option<SandboxSettings>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderTestRequest {
    provider_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillsQuery {
    workspace_root: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateThreadRequest {
    title: Option<String>,
    workspace_root: Option<PathBuf>,
    project_id: Option<Uuid>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListQuery {
    #[serde(default)]
    include_archived: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateThreadRequest {
    title: Option<String>,
    #[serde(default)]
    project_id: PatchValue<Uuid>,
    archived: Option<bool>,
    #[serde(default)]
    archived_at: PatchValue<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectRequest {
    name: String,
    workspace_root: Option<PathBuf>,
    pinned: Option<bool>,
    sort_order: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProjectRequest {
    name: Option<String>,
    #[serde(default)]
    workspace_root: PatchValue<PathBuf>,
    pinned: Option<bool>,
    sort_order: Option<i64>,
}

#[derive(Debug)]
enum PatchValue<T> {
    Missing,
    Null,
    Value(T),
}

impl<T> Default for PatchValue<T> {
    fn default() -> Self {
        Self::Missing
    }
}

impl<'de, T> Deserialize<'de> for PatchValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendMessageRequest {
    content: String,
    #[serde(default)]
    source_paths: Vec<PathBuf>,
    #[serde(default)]
    skill_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserCommandRequest {
    action: String,
    url: Option<String>,
    selector: Option<String>,
    text: Option<String>,
    clear_first: Option<bool>,
    condition: Option<String>,
    timeout_ms: Option<u64>,
    expected_filename: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelAgentTurnRequest {
    turn_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpawnSubagentRunRequest {
    name: String,
    input: String,
    parent_turn_id: Option<Uuid>,
    depth: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentInputRequest {
    input: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WaitSubagentRunRequest {
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpToolCallRequest {
    tool_name: String,
    arguments: Value,
    thread_id: Uuid,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalDecisionRequest {
    approved: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalDecisionResponse {
    accepted: bool,
    executed: bool,
}

#[derive(Debug, Deserialize)]
struct ApprovalQuery {
    status: Option<ApprovalStatus>,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    since: Option<i64>,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalStartResponse {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalCancelResponse {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalSessionResponse {
    session_id: Uuid,
    thread_id: Uuid,
    status: &'static str,
    cwd: PathBuf,
    shell: String,
    process_id: Option<u32>,
    started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TerminalEventKind {
    Started,
    Stdout,
    Stderr,
    Finished,
    Cancelled,
    Error,
}

impl TerminalEventKind {
    fn sse_event_name(self) -> &'static str {
        match self {
            TerminalEventKind::Started => "terminal_started",
            TerminalEventKind::Stdout => "terminal_stdout",
            TerminalEventKind::Stderr => "terminal_stderr",
            TerminalEventKind::Finished => "terminal_finished",
            TerminalEventKind::Cancelled => "terminal_cancelled",
            TerminalEventKind::Error => "terminal_error",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalEvent {
    id: Uuid,
    thread_id: Uuid,
    command_id: Uuid,
    seq: u64,
    created_at: DateTime<Utc>,
    #[serde(rename = "type")]
    kind: TerminalEventKind,
    command: Option<String>,
    cwd: Option<String>,
    data: Option<String>,
    exit_code: Option<i32>,
    success: Option<bool>,
    message: Option<String>,
}

#[derive(Debug, Default)]
struct TerminalEventFields {
    command: Option<String>,
    cwd: Option<String>,
    data: Option<String>,
    exit_code: Option<i32>,
    success: Option<bool>,
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspacePathQuery {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiffRevertRequest {
    path: String,
    #[serde(default)]
    confirm: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceDiffHunkAction {
    Stage,
    Unstage,
    Discard,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiffHunkActionRequest {
    path: String,
    scope: WorkspaceDiffScope,
    patch: String,
    action: WorkspaceDiffHunkAction,
    #[serde(default)]
    confirm: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiffActionResponse {
    path: PathBuf,
    diff: WorkspaceDiff,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextStatusResponse {
    budget: opentopia_core::ContextBudget,
    latest_summary: Option<ContextSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextCompactRequest {
    summary: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrajectoryExport {
    exported_at: DateTime<Utc>,
    thread: opentopia_core::Thread,
    messages: Vec<Message>,
    events: Vec<AgentEvent>,
    approvals: Vec<Approval>,
    artifacts: Vec<Artifact>,
    workspace_diff: Option<WorkspaceDiff>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct McpServerView {
    server: McpServerConfig,
    status: McpServerStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServerRequest {
    name: String,
    command: String,
    args: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    env_keys: Option<Vec<String>>,
    timeout_ms: Option<u64>,
    enabled: Option<bool>,
}

impl McpServerRequest {
    fn into_config(self) -> Result<McpServerConfig, ApiError> {
        let name = self.name.trim();
        let command = self.command.trim();
        if name.is_empty() {
            return Err(ApiError::bad_request("MCP server name cannot be empty"));
        }
        if command.is_empty() {
            return Err(ApiError::bad_request("MCP command cannot be empty"));
        }
        let mut config = McpServerConfig::new(name.to_string(), command.to_string());
        config.args = self.args.unwrap_or_default();
        config.cwd = self.cwd;
        config.env_keys = self.env_keys.unwrap_or_default();
        config.timeout_ms = self.timeout_ms.unwrap_or(30_000).clamp(1_000, 300_000);
        config.enabled = self.enabled.unwrap_or(true);
        Ok(config)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServerPatchRequest {
    name: Option<String>,
    command: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<PathBuf>,
    clear_cwd: Option<bool>,
    env_keys: Option<Vec<String>>,
    timeout_ms: Option<u64>,
    enabled: Option<bool>,
}

impl McpServerPatchRequest {
    fn apply(self, config: &mut McpServerConfig) -> Result<(), ApiError> {
        if let Some(name) = self.name {
            let name = name.trim();
            if name.is_empty() {
                return Err(ApiError::bad_request("MCP server name cannot be empty"));
            }
            config.name = name.to_string();
        }
        if let Some(command) = self.command {
            let command = command.trim();
            if command.is_empty() {
                return Err(ApiError::bad_request("MCP command cannot be empty"));
            }
            config.command = command.to_string();
        }
        if let Some(args) = self.args {
            config.args = args;
        }
        if self.clear_cwd.unwrap_or(false) {
            config.cwd = None;
        } else if let Some(cwd) = self.cwd {
            config.cwd = Some(cwd);
        }
        if let Some(env_keys) = self.env_keys {
            config.env_keys = env_keys;
        }
        if let Some(timeout_ms) = self.timeout_ms {
            config.timeout_ms = timeout_ms.clamp(1_000, 300_000);
        }
        if let Some(enabled) = self.enabled {
            config.enabled = enabled;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadMcpServerRequest {
    enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadMcpServerView {
    server: McpServerConfig,
    binding: Option<ThreadMcpServer>,
    enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteResponse {
    deleted: bool,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }

    fn gateway_timeout(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        let status = value
            .downcast_ref::<StoreError>()
            .map(|error| match error {
                StoreError::DuplicateWorkspace(_) => StatusCode::CONFLICT,
                StoreError::ProjectNotFound(_) => StatusCode::NOT_FOUND,
                StoreError::EmptyProjectName
                | StoreError::EmptyThreadTitle
                | StoreError::EmptyWorkspaceRoot
                | StoreError::ProjectHasNoWorkspace(_)
                | StoreError::ProjectWorkspaceInUse(_) => StatusCode::BAD_REQUEST,
            })
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl From<opentopia_core::mcp_host::McpHostError> for ApiError {
    fn from(value: opentopia_core::mcp_host::McpHostError) -> Self {
        let status = match &value {
            opentopia_core::mcp_host::McpHostError::ServerNotFound(_)
            | opentopia_core::mcp_host::McpHostError::ToolNotFound(_) => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: std::io::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_patch_distinguishes_missing_null_and_value_workspace() {
        let missing: UpdateProjectRequest =
            serde_json::from_value(json!({})).expect("deserialize missing workspace");
        assert!(matches!(missing.workspace_root, PatchValue::Missing));

        let null: UpdateProjectRequest = serde_json::from_value(json!({
            "workspaceRoot": null,
        }))
        .expect("deserialize null workspace");
        assert!(matches!(null.workspace_root, PatchValue::Null));

        let value: UpdateProjectRequest = serde_json::from_value(json!({
            "workspaceRoot": "J:\\Project\\OpenTopia",
            "sortOrder": 3,
        }))
        .expect("deserialize workspace value");
        assert!(matches!(
            value.workspace_root,
            PatchValue::Value(path) if path == PathBuf::from(r"J:\Project\OpenTopia")
        ));
        assert_eq!(value.sort_order, Some(3));
    }

    #[test]
    fn thread_requests_use_camel_case_project_and_archive_fields() {
        let project_id = Uuid::new_v4();
        let create: CreateThreadRequest = serde_json::from_value(json!({
            "projectId": project_id,
        }))
        .expect("deserialize create thread");
        assert_eq!(create.project_id, Some(project_id));

        let missing_project: UpdateThreadRequest =
            serde_json::from_value(json!({})).expect("deserialize missing project patch");
        assert!(matches!(missing_project.project_id, PatchValue::Missing));

        let assign: UpdateThreadRequest = serde_json::from_value(json!({
            "projectId": project_id,
        }))
        .expect("deserialize project assignment");
        assert!(matches!(
            assign.project_id,
            PatchValue::Value(value) if value == project_id
        ));

        let detach: UpdateThreadRequest = serde_json::from_value(json!({
            "projectId": null,
        }))
        .expect("deserialize project detachment");
        assert!(matches!(detach.project_id, PatchValue::Null));

        let archive: UpdateThreadRequest = serde_json::from_value(json!({
            "archivedAt": Utc::now().to_rfc3339(),
        }))
        .expect("deserialize archive thread");
        assert!(matches!(archive.archived_at, PatchValue::Value(_)));

        let restore: UpdateThreadRequest = serde_json::from_value(json!({
            "archivedAt": null,
        }))
        .expect("deserialize restore thread");
        assert!(matches!(restore.archived_at, PatchValue::Null));
    }

    #[test]
    fn store_errors_map_to_client_http_statuses() {
        let duplicate = ApiError::from(anyhow::Error::new(StoreError::DuplicateWorkspace(
            "j:/project/opentopia".to_string(),
        )));
        assert_eq!(duplicate.status, StatusCode::CONFLICT);

        let missing = ApiError::from(anyhow::Error::new(StoreError::ProjectNotFound(
            Uuid::new_v4(),
        )));
        assert_eq!(missing.status, StatusCode::NOT_FOUND);

        let empty = ApiError::from(anyhow::Error::new(StoreError::EmptyProjectName));
        assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn legacy_direct_tool_commands_are_not_agent_messages() {
        assert_eq!(legacy_direct_tool_command("/run cargo test"), Some("/run"));
        assert_eq!(
            legacy_direct_tool_command("  /READ src/lib.rs"),
            Some("/read")
        );
        assert_eq!(legacy_direct_tool_command("/run"), Some("/run"));
        assert_eq!(legacy_direct_tool_command("/runner status"), None);
        assert_eq!(legacy_direct_tool_command("Please /run the tests"), None);
    }
}
