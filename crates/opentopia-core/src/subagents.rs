use crate::model_context::CompiledModelContext;
use crate::provider::ModelConversationMessage;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{broadcast, mpsc, watch, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

impl SubagentRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::TimedOut
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            other => anyhow::bail!("unknown subagent run status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRun {
    pub id: Uuid,
    pub parent_thread_id: Uuid,
    pub parent_turn_id: Uuid,
    /// Stable canonical identity inside the root task tree (for example `/root/research`).
    pub agent_path: String,
    /// Canonical path of the agent that created this agent.
    pub parent_agent_path: String,
    pub name: String,
    /// Agent profile selected from the built-ins or `.codex/agents/*.toml`.
    pub agent_type: String,
    pub input: String,
    /// `none`, `all`, or a positive decimal number of parent turns.
    pub fork_turns: String,
    /// Most recent task message, including a follow-up that restarted an idle agent.
    pub last_task_message: String,
    pub depth: u8,
    pub status: SubagentRunStatus,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// Frozen at spawn time and intentionally omitted from task-list payloads.
    #[serde(skip)]
    pub initial_conversation: Vec<ModelConversationMessage>,
    /// The parent's compiled instructions at the fork point. Child-specific
    /// profile instructions are appended later as a branch suffix.
    #[serde(skip)]
    pub initial_model_context: Option<CompiledModelContext>,
}

#[derive(Debug, Clone)]
pub struct SpawnSubagentRequest {
    pub parent_thread_id: Uuid,
    pub parent_turn_id: Uuid,
    pub parent_agent_path: String,
    pub name: String,
    pub agent_type: String,
    pub input: String,
    pub fork_turns: String,
    pub depth: u8,
    pub initial_conversation: Vec<ModelConversationMessage>,
    pub initial_model_context: Option<CompiledModelContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentScope {
    pub thread_id: Uuid,
    pub parent_turn_id: Uuid,
    pub depth: u8,
    pub agent_path: String,
}

#[derive(Debug, Clone)]
pub struct SubagentSchedulerConfig {
    pub max_concurrency_per_parent: usize,
    pub max_threads: usize,
    pub max_depth: u8,
}

impl Default for SubagentSchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrency_per_parent: 6,
            max_threads: 6,
            max_depth: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    pub run: SubagentRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentMailboxMessageKind {
    Message,
    Completion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMailboxMessage {
    pub from_agent_path: String,
    pub to_agent_path: String,
    pub kind: AgentMailboxMessageKind,
    pub message: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDelivery {
    pub target_id: Option<Uuid>,
    pub agent_path: String,
    pub queued: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentWaitActivity {
    pub agents: Vec<SubagentRun>,
    pub messages: Vec<AgentMailboxMessage>,
}

#[derive(Debug, Error)]
pub enum SubagentError {
    #[error("subagent name cannot be empty")]
    EmptyName,
    #[error("subagent input cannot be empty")]
    EmptyInput,
    #[error("invalid agent task name `{0}`; use lowercase letters, digits, and underscores")]
    InvalidTaskName(String),
    #[error("invalid fork_turns `{0}`; expected `none`, `all`, or a positive integer")]
    InvalidForkTurns(String),
    #[error("agent path already exists: {0}")]
    DuplicatePath(String),
    #[error("agent thread limit reached for root task (maximum {maximum})")]
    MaximumThreads { maximum: usize },
    #[error("subagent depth {actual} exceeds maximum {maximum}")]
    MaximumDepth { actual: u8, maximum: u8 },
    #[error("subagent run not found: {0}")]
    NotFound(Uuid),
    #[error("subagent run is already terminal: {0}")]
    AlreadyTerminal(Uuid),
    #[error("cannot restore an active subagent run: {0}")]
    CannotRestoreActive(Uuid),
    #[error("subagent input channel is closed: {0}")]
    InputClosed(Uuid),
    #[error("timed out waiting for subagent run: {0}")]
    WaitTimedOut(Uuid),
}

#[async_trait]
pub trait SubagentExecutor: Send + Sync + 'static {
    async fn execute(
        &self,
        run: SubagentRun,
        input: mpsc::UnboundedReceiver<String>,
        cancellation: CancellationToken,
    ) -> anyhow::Result<String>;
}

pub trait SubagentObserver: Send + Sync + 'static {
    fn on_update(&self, run: &SubagentRun);
}

#[derive(Debug, Default)]
pub struct NoopSubagentObserver;

impl SubagentObserver for NoopSubagentObserver {
    fn on_update(&self, _run: &SubagentRun) {}
}

struct RunControl {
    run: Mutex<SubagentRun>,
    cancellation: CancellationToken,
    input: mpsc::UnboundedSender<String>,
    updates: watch::Sender<SubagentRun>,
}

struct SchedulerInner {
    config: SubagentSchedulerConfig,
    executor: Arc<dyn SubagentExecutor>,
    observer: Arc<dyn SubagentObserver>,
    runs: Mutex<HashMap<Uuid, Arc<RunControl>>>,
    queued_messages: Mutex<HashMap<Uuid, Vec<String>>>,
    groups: Mutex<HashMap<Uuid, Arc<Semaphore>>>,
    events: broadcast::Sender<SubagentEvent>,
    mailboxes: Mutex<HashMap<(Uuid, String), Vec<AgentMailboxMessage>>>,
    mailbox_events: broadcast::Sender<(Uuid, AgentMailboxMessage)>,
}

#[derive(Clone)]
pub struct SubagentScheduler {
    inner: Arc<SchedulerInner>,
}

impl SubagentScheduler {
    pub fn new(
        config: SubagentSchedulerConfig,
        executor: Arc<dyn SubagentExecutor>,
        observer: Arc<dyn SubagentObserver>,
    ) -> Self {
        let (events, _) = broadcast::channel(256);
        let (mailbox_events, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(SchedulerInner {
                config,
                executor,
                observer,
                runs: Mutex::new(HashMap::new()),
                queued_messages: Mutex::new(HashMap::new()),
                groups: Mutex::new(HashMap::new()),
                events,
                mailboxes: Mutex::new(HashMap::new()),
                mailbox_events,
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SubagentEvent> {
        self.inner.events.subscribe()
    }

    /// Re-register a persisted, non-running agent identity after process restart.
    pub fn restore(&self, run: SubagentRun) -> Result<(), SubagentError> {
        if !run.status.is_terminal() {
            return Err(SubagentError::CannotRestoreActive(run.id));
        }
        let (input, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        let (updates, _) = watch::channel(run.clone());
        let control = Arc::new(RunControl {
            run: Mutex::new(run.clone()),
            cancellation: CancellationToken::new(),
            input,
            updates,
        });
        let mut controls = self
            .inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned");
        if controls.contains_key(&run.id) {
            return Ok(());
        }
        if controls.values().any(|existing| {
            let existing = existing.run.lock().expect("subagent run mutex poisoned");
            existing.parent_thread_id == run.parent_thread_id
                && existing.agent_path == run.agent_path
        }) {
            return Err(SubagentError::DuplicatePath(run.agent_path));
        }
        controls.insert(run.id, control);
        Ok(())
    }

    pub fn spawn(&self, request: SpawnSubagentRequest) -> Result<SubagentRun, SubagentError> {
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err(SubagentError::EmptyName);
        }
        let input_text = request.input.trim().to_string();
        if input_text.is_empty() {
            return Err(SubagentError::EmptyInput);
        }
        if !is_valid_task_name(&name) {
            return Err(SubagentError::InvalidTaskName(name));
        }
        let fork_turns = normalize_fork_turns(&request.fork_turns)?;
        if request.depth > self.inner.config.max_depth {
            return Err(SubagentError::MaximumDepth {
                actual: request.depth,
                maximum: self.inner.config.max_depth,
            });
        }

        let parent_agent_path = normalize_agent_path(&request.parent_agent_path);
        let agent_path = format!("{parent_agent_path}/{name}");
        let run = SubagentRun {
            id: Uuid::new_v4(),
            parent_thread_id: request.parent_thread_id,
            parent_turn_id: request.parent_turn_id,
            agent_path,
            parent_agent_path,
            name,
            agent_type: normalize_agent_type(&request.agent_type),
            input: input_text,
            fork_turns,
            last_task_message: request.input.trim().to_string(),
            depth: request.depth,
            status: SubagentRunStatus::Queued,
            result: None,
            error: None,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            initial_conversation: request.initial_conversation,
            initial_model_context: request.initial_model_context,
        };
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let (updates, _) = watch::channel(run.clone());
        let control = Arc::new(RunControl {
            run: Mutex::new(run.clone()),
            cancellation: CancellationToken::new(),
            input: input_tx,
            updates,
        });
        {
            let mut controls = self
                .inner
                .runs
                .lock()
                .expect("subagent runs mutex poisoned");
            if controls.values().any(|control| {
                let existing = control.run.lock().expect("subagent run mutex poisoned");
                existing.parent_thread_id == run.parent_thread_id
                    && existing.agent_path == run.agent_path
            }) {
                return Err(SubagentError::DuplicatePath(run.agent_path.clone()));
            }
            let active = controls
                .values()
                .filter(|control| {
                    let existing = control.run.lock().expect("subagent run mutex poisoned");
                    existing.parent_thread_id == run.parent_thread_id
                        && !existing.status.is_terminal()
                })
                .count();
            if active >= self.inner.config.max_threads.max(1) {
                return Err(SubagentError::MaximumThreads {
                    maximum: self.inner.config.max_threads.max(1),
                });
            }
            controls.insert(run.id, control.clone());
        }
        self.publish(&run);

        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.execute(control, input_rx).await;
        });
        Ok(run)
    }

    pub fn get(&self, run_id: Uuid) -> Option<SubagentRun> {
        self.control(run_id).map(|control| {
            control
                .run
                .lock()
                .expect("subagent run mutex poisoned")
                .clone()
        })
    }

    pub fn list_for_thread(&self, thread_id: Uuid) -> Vec<SubagentRun> {
        let controls = self
            .inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut runs = controls
            .into_iter()
            .filter_map(|control| {
                let run = control
                    .run
                    .lock()
                    .expect("subagent run mutex poisoned")
                    .clone();
                (run.parent_thread_id == thread_id).then_some(run)
            })
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        runs
    }

    pub fn list_scoped(&self, scope: SubagentScope, path_prefix: Option<&str>) -> Vec<SubagentRun> {
        let prefix = path_prefix.map(normalize_agent_path);
        self.list_for_thread(scope.thread_id)
            .into_iter()
            .filter(|run| {
                prefix
                    .as_ref()
                    .map(|prefix| run.agent_path.starts_with(prefix))
                    .unwrap_or(true)
            })
            .collect()
    }

    pub fn list_descendants_scoped(&self, scope: &SubagentScope) -> Vec<SubagentRun> {
        let parent_path = normalize_agent_path(&scope.agent_path);
        let descendant_prefix = format!("{}/", parent_path.trim_end_matches('/'));
        self.list_for_thread(scope.thread_id)
            .into_iter()
            .filter(|run| run.agent_path.starts_with(&descendant_prefix))
            .collect()
    }

    pub fn resolve_scoped(
        &self,
        scope: SubagentScope,
        target: &str,
    ) -> Result<SubagentRun, SubagentError> {
        let target = target.trim();
        let run = Uuid::parse_str(target)
            .ok()
            .and_then(|id| self.get(id))
            .or_else(|| {
                let path = resolve_target_path(&scope.agent_path, target);
                self.list_for_thread(scope.thread_id)
                    .into_iter()
                    .find(|run| run.agent_path == path)
            })
            .ok_or_else(|| SubagentError::NotFound(Uuid::nil()))?;
        if run.parent_thread_id != scope.thread_id {
            return Err(SubagentError::NotFound(run.id));
        }
        Ok(run)
    }

    pub fn send_input(&self, run_id: Uuid, input: String) -> Result<(), SubagentError> {
        let control = self
            .control(run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        if control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .status
            .is_terminal()
        {
            return Err(SubagentError::AlreadyTerminal(run_id));
        }
        if input.trim().is_empty() {
            return Err(SubagentError::EmptyInput);
        }
        control
            .input
            .send(input)
            .map_err(|_| SubagentError::InputClosed(run_id))
    }

    pub fn send_input_scoped(
        &self,
        scope: SubagentScope,
        run_id: Uuid,
        input: String,
    ) -> Result<(), SubagentError> {
        self.ensure_visible(scope, run_id)?;
        self.send_input(run_id, input)
    }

    /// Queue a message for an agent without starting a new turn when the agent is idle.
    pub fn send_message_scoped(
        &self,
        scope: SubagentScope,
        target: &str,
        message: String,
    ) -> Result<AgentMessageDelivery, SubagentError> {
        if message.trim().is_empty() {
            return Err(SubagentError::EmptyInput);
        }
        let target_path = resolve_target_path(&scope.agent_path, target);
        if target_path == "/root" {
            self.queue_mailbox_message(
                scope.thread_id,
                AgentMailboxMessage {
                    from_agent_path: scope.agent_path,
                    to_agent_path: target_path.clone(),
                    kind: AgentMailboxMessageKind::Message,
                    message,
                    created_at: Utc::now(),
                },
            );
            return Ok(AgentMessageDelivery {
                target_id: None,
                agent_path: target_path,
                queued: true,
            });
        }
        let run = self.resolve_scoped(scope.clone(), &target_path)?;
        let mut queued = false;
        let rendered = render_agent_message(&scope.agent_path, &message);
        if run.status.is_terminal() {
            self.inner
                .queued_messages
                .lock()
                .expect("subagent mailbox mutex poisoned")
                .entry(run.id)
                .or_default()
                .push(rendered);
            queued = true;
        } else {
            match self.send_input(run.id, rendered.clone()) {
                Ok(()) => {}
                Err(SubagentError::InputClosed(_) | SubagentError::AlreadyTerminal(_)) => {
                    self.inner
                        .queued_messages
                        .lock()
                        .expect("subagent mailbox mutex poisoned")
                        .entry(run.id)
                        .or_default()
                        .push(rendered);
                    queued = true;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(AgentMessageDelivery {
            target_id: Some(run.id),
            agent_path: run.agent_path,
            queued,
        })
    }

    /// Deliver a task and start a fresh turn if the target agent is idle.
    pub fn followup_task_scoped(
        &self,
        scope: SubagentScope,
        target: &str,
        message: String,
    ) -> Result<SubagentRun, SubagentError> {
        if message.trim().is_empty() {
            return Err(SubagentError::EmptyInput);
        }
        let current = self.resolve_scoped(scope.clone(), target)?;
        if !current.status.is_terminal() {
            self.send_input(current.id, message.clone())?;
            return Ok(self.get(current.id).expect("active agent disappeared"));
        }

        let mut run = current;
        run.parent_turn_id = scope.parent_turn_id;
        run.last_task_message = message.clone();
        run.status = SubagentRunStatus::Queued;
        run.result = None;
        run.error = None;
        run.started_at = None;
        run.completed_at = None;
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        let pending = self
            .inner
            .queued_messages
            .lock()
            .expect("subagent mailbox mutex poisoned")
            .remove(&run.id)
            .unwrap_or_default();
        for queued in pending {
            let _ = input_tx.send(queued);
        }
        let (updates, _) = watch::channel(run.clone());
        let control = Arc::new(RunControl {
            run: Mutex::new(run.clone()),
            cancellation: CancellationToken::new(),
            input: input_tx,
            updates,
        });
        self.inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .insert(run.id, control.clone());
        self.publish(&run);
        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.execute(control, input_rx).await;
        });
        Ok(run)
    }

    pub fn cancel(&self, run_id: Uuid) -> Result<(), SubagentError> {
        let control = self
            .control(run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        if control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .status
            .is_terminal()
        {
            return Err(SubagentError::AlreadyTerminal(run_id));
        }
        control.cancellation.cancel();
        self.cancel_parent(run_id);
        Ok(())
    }

    pub fn cancel_scoped(&self, scope: SubagentScope, run_id: Uuid) -> Result<(), SubagentError> {
        self.ensure_visible(scope, run_id)?;
        self.cancel(run_id)
    }

    pub fn cancel_parent(&self, parent_turn_id: Uuid) -> usize {
        let controls = self
            .inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut cancelled = 0usize;
        let mut parents = vec![parent_turn_id];
        let mut visited = std::collections::HashSet::new();
        while let Some(parent) = parents.pop() {
            if !visited.insert(parent) {
                continue;
            }
            for control in &controls {
                let run = control
                    .run
                    .lock()
                    .expect("subagent run mutex poisoned")
                    .clone();
                if run.parent_turn_id == parent && !run.status.is_terminal() {
                    control.cancellation.cancel();
                    cancelled += 1;
                    parents.push(run.id);
                }
            }
        }
        cancelled
    }

    pub async fn wait(
        &self,
        run_id: Uuid,
        timeout: Duration,
    ) -> Result<SubagentRun, SubagentError> {
        let control = self
            .control(run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        let mut updates = control.updates.subscribe();
        let future = async {
            loop {
                let run = updates.borrow().clone();
                if run.status.is_terminal() {
                    return run;
                }
                if updates.changed().await.is_err() {
                    return updates.borrow().clone();
                }
            }
        };
        tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| SubagentError::WaitTimedOut(run_id))
    }

    pub async fn wait_scoped(
        &self,
        scope: SubagentScope,
        run_id: Uuid,
        timeout: Duration,
    ) -> Result<SubagentRun, SubagentError> {
        self.ensure_visible(scope, run_id)?;
        self.wait(run_id, timeout).await
    }

    pub async fn wait_for_activity_scoped(
        &self,
        scope: SubagentScope,
        timeout: Duration,
    ) -> Result<AgentWaitActivity, SubagentError> {
        let mut events = self.subscribe();
        let mut mailbox_events = self.inner.mailbox_events.subscribe();
        let messages = self.drain_mailbox(scope.thread_id, &scope.agent_path);
        if !messages.is_empty() {
            return Ok(AgentWaitActivity {
                agents: Vec::new(),
                messages,
            });
        }
        let snapshot = self.list_scoped(scope.clone(), None);
        if snapshot.iter().all(|run| run.status.is_terminal()) {
            return Ok(AgentWaitActivity {
                agents: snapshot,
                messages: Vec::new(),
            });
        }
        let future = async {
            loop {
                tokio::select! {
                    event = mailbox_events.recv() => match event {
                        Ok((thread_id, message))
                            if thread_id == scope.thread_id
                                && message.to_agent_path == normalize_agent_path(&scope.agent_path) =>
                        {
                            return AgentWaitActivity {
                                agents: Vec::new(),
                                messages: self.drain_mailbox(scope.thread_id, &scope.agent_path),
                            };
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {}
                    },
                    event = events.recv() => match event {
                        Ok(event)
                            if event.run.parent_thread_id == scope.thread_id
                                && event.run.status.is_terminal() =>
                        {
                            return AgentWaitActivity {
                                agents: vec![event.run],
                                messages: self.drain_mailbox(scope.thread_id, &scope.agent_path),
                            };
                        }
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let current = self.list_scoped(scope.clone(), None);
                            if current.iter().any(|run| run.status.is_terminal()) {
                                return AgentWaitActivity {
                                    agents: current,
                                    messages: self.drain_mailbox(scope.thread_id, &scope.agent_path),
                                };
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            return AgentWaitActivity {
                                agents: Vec::new(),
                                messages: self.drain_mailbox(scope.thread_id, &scope.agent_path),
                            };
                        }
                    },
                }
            }
        };
        tokio::time::timeout(timeout, future)
            .await
            .map_err(|_| SubagentError::WaitTimedOut(Uuid::nil()))
    }

    pub fn drain_mailbox_scoped(&self, scope: &SubagentScope) -> Vec<AgentMailboxMessage> {
        self.drain_mailbox(scope.thread_id, &scope.agent_path)
    }

    pub fn mailbox_snapshot_scoped(&self, scope: &SubagentScope) -> Vec<AgentMailboxMessage> {
        self.inner
            .mailboxes
            .lock()
            .expect("agent mailboxes mutex poisoned")
            .get(&(scope.thread_id, normalize_agent_path(&scope.agent_path)))
            .cloned()
            .unwrap_or_default()
    }

    pub fn acknowledge_mailbox_scoped(
        &self,
        scope: &SubagentScope,
        delivered: &[AgentMailboxMessage],
    ) {
        if delivered.is_empty() {
            return;
        }
        let key = (scope.thread_id, normalize_agent_path(&scope.agent_path));
        let mut mailboxes = self
            .inner
            .mailboxes
            .lock()
            .expect("agent mailboxes mutex poisoned");
        let Some(messages) = mailboxes.get_mut(&key) else {
            return;
        };
        messages.retain(|message| !delivered.contains(message));
        if messages.is_empty() {
            mailboxes.remove(&key);
        }
    }

    pub fn drain_mailbox_from_scoped(
        &self,
        scope: &SubagentScope,
        from_agent_path: &str,
    ) -> Vec<AgentMailboxMessage> {
        let key = (scope.thread_id, normalize_agent_path(&scope.agent_path));
        let from_agent_path = normalize_agent_path(from_agent_path);
        let mut mailboxes = self
            .inner
            .mailboxes
            .lock()
            .expect("agent mailboxes mutex poisoned");
        let Some(messages) = mailboxes.remove(&key) else {
            return Vec::new();
        };
        let (matching, remaining): (Vec<_>, Vec<_>) = messages
            .into_iter()
            .partition(|message| normalize_agent_path(&message.from_agent_path) == from_agent_path);
        if !remaining.is_empty() {
            mailboxes.insert(key, remaining);
        }
        matching
    }

    fn ensure_visible(&self, scope: SubagentScope, run_id: Uuid) -> Result<(), SubagentError> {
        let run = self.get(run_id).ok_or(SubagentError::NotFound(run_id))?;
        if run.parent_thread_id != scope.thread_id {
            // Do not disclose whether a UUID belongs to another root task.
            return Err(SubagentError::NotFound(run_id));
        }
        Ok(())
    }

    async fn execute(&self, control: Arc<RunControl>, input: mpsc::UnboundedReceiver<String>) {
        let parent_turn_id = control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .parent_turn_id;
        let semaphore = self
            .inner
            .groups
            .lock()
            .expect("subagent groups mutex poisoned")
            .entry(parent_turn_id)
            .or_insert_with(|| {
                Arc::new(Semaphore::new(
                    self.inner.config.max_concurrency_per_parent.max(1),
                ))
            })
            .clone();
        let permit = tokio::select! {
            _ = control.cancellation.cancelled() => {
                self.finish(&control, SubagentRunStatus::Cancelled, None, None);
                return;
            }
            permit = semaphore.acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => {
                    self.finish(&control, SubagentRunStatus::Failed, None, Some("subagent queue closed".to_string()));
                    return;
                }
            }
        };
        self.transition_running(&control);
        let run = control
            .run
            .lock()
            .expect("subagent run mutex poisoned")
            .clone();
        let execution = self
            .inner
            .executor
            .execute(run, input, control.cancellation.child_token());
        tokio::pin!(execution);
        tokio::select! {
            _ = control.cancellation.cancelled() => {
                self.finish(&control, SubagentRunStatus::Cancelled, None, None);
            }
            result = &mut execution => self.finish_execution_result(&control, result),
        }
        drop(permit);
    }

    fn finish_execution_result(&self, control: &RunControl, result: anyhow::Result<String>) {
        match result {
            Ok(result) => self.finish(control, SubagentRunStatus::Completed, Some(result), None),
            Err(_) if control.cancellation.is_cancelled() => {
                self.finish(control, SubagentRunStatus::Cancelled, None, None)
            }
            Err(error) => self.finish(
                control,
                SubagentRunStatus::Failed,
                None,
                Some(error.to_string()),
            ),
        }
    }

    fn transition_running(&self, control: &RunControl) {
        let run = {
            let mut run = control.run.lock().expect("subagent run mutex poisoned");
            if run.status != SubagentRunStatus::Queued {
                return;
            }
            run.status = SubagentRunStatus::Running;
            run.started_at = Some(Utc::now());
            run.clone()
        };
        control.updates.send_replace(run.clone());
        self.publish(&run);
    }

    fn finish(
        &self,
        control: &RunControl,
        status: SubagentRunStatus,
        result: Option<String>,
        error: Option<String>,
    ) {
        let run = {
            let mut run = control.run.lock().expect("subagent run mutex poisoned");
            if run.status.is_terminal() {
                return;
            }
            run.status = status;
            run.result = result;
            run.error = error;
            run.completed_at = Some(Utc::now());
            self.queue_mailbox_message(
                run.parent_thread_id,
                AgentMailboxMessage {
                    from_agent_path: run.agent_path.clone(),
                    to_agent_path: run.parent_agent_path.clone(),
                    kind: AgentMailboxMessageKind::Completion,
                    message: serde_json::to_string(&serde_json::json!({
                        "agentPath": run.agent_path,
                        "status": run.status,
                        "result": run.result,
                        "error": run.error,
                    }))
                    .unwrap_or_else(|_| "agent turn completed".to_string()),
                    created_at: Utc::now(),
                },
            );
            run.clone()
        };
        control.updates.send_replace(run.clone());
        self.publish(&run);
    }

    fn publish(&self, run: &SubagentRun) {
        self.inner.observer.on_update(run);
        let _ = self.inner.events.send(SubagentEvent { run: run.clone() });
    }

    fn queue_mailbox_message(&self, thread_id: Uuid, message: AgentMailboxMessage) {
        self.inner
            .mailboxes
            .lock()
            .expect("agent mailboxes mutex poisoned")
            .entry((thread_id, message.to_agent_path.clone()))
            .or_default()
            .push(message.clone());
        let _ = self.inner.mailbox_events.send((thread_id, message));
    }

    fn drain_mailbox(&self, thread_id: Uuid, agent_path: &str) -> Vec<AgentMailboxMessage> {
        self.inner
            .mailboxes
            .lock()
            .expect("agent mailboxes mutex poisoned")
            .remove(&(thread_id, normalize_agent_path(agent_path)))
            .unwrap_or_default()
    }

    fn control(&self, run_id: Uuid) -> Option<Arc<RunControl>> {
        self.inner
            .runs
            .lock()
            .expect("subagent runs mutex poisoned")
            .get(&run_id)
            .cloned()
    }
}

fn normalize_agent_path(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        "/root".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn resolve_target_path(current_agent_path: &str, target: &str) -> String {
    let target = target.trim();
    if Uuid::parse_str(target).is_ok() {
        target.to_string()
    } else if target.starts_with('/') {
        normalize_agent_path(target)
    } else {
        format!("{}/{}", normalize_agent_path(current_agent_path), target)
    }
}

fn render_agent_message(from_agent_path: &str, message: &str) -> String {
    format!(
        "Message from {}:\n{}",
        normalize_agent_path(from_agent_path),
        message.trim()
    )
}

fn normalize_agent_type(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        "default".to_string()
    } else {
        value.to_string()
    }
}

fn is_valid_task_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn normalize_fork_turns(value: &str) -> Result<String, SubagentError> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "none" | "all") || value.parse::<u16>().is_ok_and(|turns| turns > 0)
    {
        Ok(value)
    } else {
        Err(SubagentError::InvalidForkTurns(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestExecutor {
        active: AtomicUsize,
        peak: AtomicUsize,
        delay: Duration,
        fail: bool,
    }

    #[async_trait]
    impl SubagentExecutor for TestExecutor {
        async fn execute(
            &self,
            run: SubagentRun,
            mut input: mpsc::UnboundedReceiver<String>,
            cancellation: CancellationToken,
        ) -> anyhow::Result<String> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let result = tokio::select! {
                _ = cancellation.cancelled() => anyhow::bail!("cancelled"),
                _ = tokio::time::sleep(self.delay) => {
                    if self.fail { anyhow::bail!("failed intentionally") }
                    let mut messages = Vec::new();
                    while let Ok(message) = input.try_recv() { messages.push(message); }
                    Ok(format!("{}:{}", run.name, messages.join(",")))
                }
            };
            self.active.fetch_sub(1, Ordering::SeqCst);
            result
        }
    }

    fn scheduler(max: usize, delay: Duration) -> (SubagentScheduler, Arc<TestExecutor>) {
        let executor = Arc::new(TestExecutor {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay,
            fail: false,
        });
        let scheduler = SubagentScheduler::new(
            SubagentSchedulerConfig {
                max_concurrency_per_parent: max,
                max_threads: 16,
                max_depth: 2,
            },
            executor.clone(),
            Arc::new(NoopSubagentObserver),
        );
        (scheduler, executor)
    }

    fn request(parent: Uuid, name: &str) -> SpawnSubagentRequest {
        SpawnSubagentRequest {
            parent_thread_id: Uuid::new_v4(),
            parent_turn_id: parent,
            parent_agent_path: "/root".to_string(),
            name: name.to_string(),
            agent_type: "default".to_string(),
            input: "work".to_string(),
            fork_turns: "all".to_string(),
            depth: 1,
            initial_conversation: Vec::new(),
            initial_model_context: None,
        }
    }

    fn owning_scope(run: &SubagentRun) -> SubagentScope {
        SubagentScope {
            thread_id: run.parent_thread_id,
            parent_turn_id: run.parent_turn_id,
            depth: run.depth - 1,
            agent_path: run.parent_agent_path.clone(),
        }
    }

    #[test]
    fn mailbox_acknowledgement_preserves_messages_after_the_delivered_snapshot() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(20));
        let scope = SubagentScope {
            thread_id: Uuid::new_v4(),
            parent_turn_id: Uuid::new_v4(),
            depth: 0,
            agent_path: "/root".to_string(),
        };
        let message = |text: &str| AgentMailboxMessage {
            from_agent_path: "/root/worker".to_string(),
            to_agent_path: "/root".to_string(),
            kind: AgentMailboxMessageKind::Message,
            message: text.to_string(),
            created_at: Utc::now(),
        };
        scheduler.queue_mailbox_message(scope.thread_id, message("delivered"));
        let delivered = scheduler.mailbox_snapshot_scoped(&scope);
        scheduler.queue_mailbox_message(scope.thread_id, message("arrived later"));

        scheduler.acknowledge_mailbox_scoped(&scope, &delivered);

        let remaining = scheduler.mailbox_snapshot_scoped(&scope);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].message, "arrived later");
    }

    #[tokio::test]
    async fn queues_fairly_and_enforces_parent_concurrency() {
        let (scheduler, executor) = scheduler(2, Duration::from_millis(40));
        let parent = Uuid::new_v4();
        let runs = (0..5)
            .map(|index| {
                scheduler
                    .spawn(request(parent, &format!("run_{index}")))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for run in runs {
            assert_eq!(
                scheduler
                    .wait(run.id, Duration::from_secs(2))
                    .await
                    .unwrap()
                    .status,
                SubagentRunStatus::Completed
            );
        }
        assert_eq!(executor.peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn send_input_is_delivered_and_wait_returns_result() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(40));
        let run = scheduler.spawn(request(Uuid::new_v4(), "reader")).unwrap();
        scheduler
            .send_input(run.id, "follow-up".to_string())
            .unwrap();
        let run = scheduler
            .wait(run.id, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(run.status, SubagentRunStatus::Completed);
        assert_eq!(run.result.as_deref(), Some("reader:follow-up"));
    }

    #[tokio::test]
    async fn completed_agent_can_receive_queued_messages_and_run_a_followup_turn() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(20));
        let first = scheduler
            .spawn(request(Uuid::new_v4(), "reusable"))
            .unwrap();
        let completed = scheduler
            .wait(first.id, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(completed.status, SubagentRunStatus::Completed);

        let scope = owning_scope(&completed);
        scheduler
            .send_message_scoped(scope.clone(), &completed.agent_path, "queued".to_string())
            .unwrap();
        let restarted = scheduler
            .followup_task_scoped(scope, &completed.agent_path, "continue".to_string())
            .unwrap();
        assert_eq!(restarted.id, completed.id);
        assert_eq!(restarted.last_task_message, "continue");
        let finished = scheduler
            .wait(restarted.id, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(finished.status, SubagentRunStatus::Completed);
        assert_eq!(
            finished.result.as_deref(),
            Some("reusable:Message from /root:\nqueued")
        );
    }

    #[tokio::test]
    async fn root_mailbox_receives_direct_messages_and_completion_notifications() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(20));
        let run = scheduler
            .spawn(request(Uuid::new_v4(), "reporter"))
            .unwrap();
        let completed = scheduler
            .wait(run.id, Duration::from_secs(2))
            .await
            .unwrap();
        let child_scope = SubagentScope {
            thread_id: completed.parent_thread_id,
            parent_turn_id: completed.id,
            depth: completed.depth,
            agent_path: completed.agent_path.clone(),
        };
        scheduler
            .send_message_scoped(child_scope, "/root", "evidence".to_string())
            .unwrap();

        let activity = scheduler
            .wait_for_activity_scoped(owning_scope(&completed), Duration::from_secs(1))
            .await
            .unwrap();
        assert!(activity
            .messages
            .iter()
            .any(|message| message.kind == AgentMailboxMessageKind::Completion));
        assert!(activity.messages.iter().any(|message| {
            message.kind == AgentMailboxMessageKind::Message && message.message == "evidence"
        }));
    }

    #[tokio::test]
    async fn scoped_operations_hide_runs_from_other_threads() {
        let (scheduler, _) = scheduler(1, Duration::from_secs(5));
        let run = scheduler.spawn(request(Uuid::new_v4(), "private")).unwrap();
        let cross_thread_scope = SubagentScope {
            thread_id: Uuid::new_v4(),
            ..owning_scope(&run)
        };

        assert!(matches!(
            scheduler.send_input_scoped(cross_thread_scope.clone(), run.id, "intrude".to_string()),
            Err(SubagentError::NotFound(id)) if id == run.id
        ));
        assert!(matches!(
            scheduler.cancel_scoped(cross_thread_scope.clone(), run.id),
            Err(SubagentError::NotFound(id)) if id == run.id
        ));
        assert!(matches!(
            scheduler
                .wait_scoped(cross_thread_scope, run.id, Duration::from_millis(5))
                .await,
            Err(SubagentError::NotFound(id)) if id == run.id
        ));

        scheduler.cancel(run.id).unwrap();
    }

    #[tokio::test]
    async fn scoped_operations_allow_peer_communication_inside_the_same_root_task() {
        let (scheduler, _) = scheduler(1, Duration::from_secs(5));
        let run = scheduler
            .spawn(request(Uuid::new_v4(), "direct_child"))
            .unwrap();
        let wrong_parent_scope = SubagentScope {
            parent_turn_id: Uuid::new_v4(),
            ..owning_scope(&run)
        };
        let wrong_depth_scope = SubagentScope {
            depth: run.depth,
            ..owning_scope(&run)
        };

        scheduler
            .send_input_scoped(wrong_parent_scope, run.id, "peer message".to_string())
            .unwrap();
        scheduler.cancel_scoped(wrong_depth_scope, run.id).unwrap();
    }

    #[tokio::test]
    async fn supports_single_and_parent_cancellation() {
        let (scheduler, _) = scheduler(2, Duration::from_secs(5));
        let parent = Uuid::new_v4();
        let one = scheduler.spawn(request(parent, "one")).unwrap();
        let two = scheduler.spawn(request(parent, "two")).unwrap();
        let mut nested_request = request(one.id, "nested");
        nested_request.depth = 2;
        nested_request.parent_agent_path = one.agent_path.clone();
        let nested = scheduler.spawn(nested_request).unwrap();
        scheduler.cancel(one.id).unwrap();
        assert_eq!(scheduler.cancel_parent(parent), 3);
        assert_eq!(
            scheduler
                .wait(one.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Cancelled
        );
        assert_eq!(
            scheduler
                .wait(two.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Cancelled
        );
        assert_eq!(
            scheduler
                .wait(nested.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn long_running_execution_completes_and_excessive_depth_is_rejected() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(100));
        let run = scheduler.spawn(request(Uuid::new_v4(), "slow")).unwrap();
        assert_eq!(
            scheduler
                .wait(run.id, Duration::from_secs(2))
                .await
                .unwrap()
                .status,
            SubagentRunStatus::Completed
        );
        let mut deep = request(Uuid::new_v4(), "deep");
        deep.depth = 3;
        assert!(matches!(
            scheduler.spawn(deep),
            Err(SubagentError::MaximumDepth { .. })
        ));
    }

    #[tokio::test]
    async fn wait_has_its_own_timeout_and_terminal_runs_reject_input() {
        let (scheduler, _) = scheduler(1, Duration::from_millis(100));
        let run = scheduler.spawn(request(Uuid::new_v4(), "wait")).unwrap();
        assert!(matches!(
            scheduler.wait(run.id, Duration::from_millis(5)).await,
            Err(SubagentError::WaitTimedOut(_))
        ));
        scheduler.cancel(run.id).unwrap();
        scheduler
            .wait(run.id, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(
            scheduler.send_input(run.id, "late".to_string()),
            Err(SubagentError::AlreadyTerminal(_))
        ));
    }
}
