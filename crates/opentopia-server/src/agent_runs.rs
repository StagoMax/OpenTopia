use async_trait::async_trait;
use opentopia_core::collaboration::{
    AgentMailboxMessage, AgentMailboxNotifier, AgentRunCommand, AgentRunScheduler,
    AgentRunSchedulerError, AgentThreadId, AgentTurnId, CollaborationRegistry,
    CollaborationSessionId, RuntimeWorkspaceModeV1, SqliteCollaborationRepository,
};
use opentopia_core::{TurnInbox, TurnInboxItem};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait AgentRunExecutor: Send + Sync {
    async fn execute(&self, command: AgentRunCommand, cancellation: CancellationToken);
}

#[derive(Clone)]
struct ActiveRun {
    turn_id: AgentTurnId,
    cancellation: CancellationToken,
}

/// Application scheduler at the whole-Agent-Run boundary. It owns admission,
/// queueing and cancellation only; the executor remains the single owner of
/// AgentTurnDriver and all model/tool/safe-point behavior.
pub struct ServerAgentRunScheduler {
    sender: mpsc::UnboundedSender<AgentRunCommand>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<AgentRunCommand>>>,
    repository: Arc<SqliteCollaborationRepository>,
    turn_inbox: Arc<dyn TurnInbox>,
    global_slots: Arc<Semaphore>,
    session_slots: Mutex<HashMap<CollaborationSessionId, Arc<Semaphore>>>,
    shared_workspace_slots: Mutex<HashMap<String, Arc<Semaphore>>>,
    active: Mutex<HashMap<AgentThreadId, ActiveRun>>,
    cancelled_before_start: Mutex<HashSet<AgentTurnId>>,
}

impl ServerAgentRunScheduler {
    pub fn new(
        repository: Arc<SqliteCollaborationRepository>,
        turn_inbox: Arc<dyn TurnInbox>,
        max_active_runs: usize,
    ) -> Arc<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        Arc::new(Self {
            sender,
            receiver: Mutex::new(Some(receiver)),
            repository,
            turn_inbox,
            global_slots: Arc::new(Semaphore::new(max_active_runs.max(1))),
            session_slots: Mutex::new(HashMap::new()),
            shared_workspace_slots: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            cancelled_before_start: Mutex::new(HashSet::new()),
        })
    }

    pub fn start(self: &Arc<Self>, executor: Arc<dyn AgentRunExecutor>) {
        let receiver = self
            .receiver
            .lock()
            .expect("Agent run receiver lock poisoned")
            .take()
            .expect("Agent run scheduler may only be started once");
        let scheduler = self.clone();
        tokio::spawn(async move { scheduler.run(receiver, executor).await });
    }

    async fn run(
        self: Arc<Self>,
        mut receiver: mpsc::UnboundedReceiver<AgentRunCommand>,
        executor: Arc<dyn AgentRunExecutor>,
    ) {
        while let Some(command) = receiver.recv().await {
            let (session_id, agent_thread_id, agent_turn_id) = command_identity(&command);
            let session_limit = self
                .repository
                .get_session(session_id)
                .await
                .map(|session| session.policy.max_active_runs)
                .unwrap_or(1)
                .max(1);
            let session_slots = self
                .session_slots
                .lock()
                .expect("session slots lock poisoned")
                .entry(session_id)
                .or_insert_with(|| Arc::new(Semaphore::new(session_limit)))
                .clone();
            let scheduler = self.clone();
            let executor = executor.clone();
            tokio::spawn(async move {
                let Ok(global_permit) = scheduler.global_slots.clone().acquire_owned().await else {
                    return;
                };
                let Ok(session_permit) = session_slots.acquire_owned().await else {
                    return;
                };
                // Shared coordinated workspaces are conservatively serialized.
                // Isolated worktrees and read-only Agents do not take this lease.
                // This is stricter than path-level leases, but never permits two
                // undeclared writers to race on the same workspace.
                let shared_workspace = match scheduler.repository.get_thread(agent_thread_id).await
                {
                    Ok(thread) => scheduler
                        .repository
                        .get_runtime_snapshot(thread.runtime_snapshot_id)
                        .await
                        .ok(),
                    Err(_) => None,
                }
                .and_then(|snapshot| {
                    snapshot.decode().ok().and_then(|snapshot| {
                        (snapshot.workspace_mode == RuntimeWorkspaceModeV1::SharedCoordinated)
                            .then(|| snapshot.workspace_root.to_string_lossy().into_owned())
                    })
                });
                let workspace_permit = if let Some(workspace) = shared_workspace {
                    let slots = scheduler
                        .shared_workspace_slots
                        .lock()
                        .expect("shared workspace slots lock poisoned")
                        .entry(workspace)
                        .or_insert_with(|| Arc::new(Semaphore::new(1)))
                        .clone();
                    slots.acquire_owned().await.ok()
                } else {
                    None
                };
                let cancellation = CancellationToken::new();
                if scheduler
                    .cancelled_before_start
                    .lock()
                    .expect("queued cancellation lock poisoned")
                    .remove(&agent_turn_id)
                {
                    cancellation.cancel();
                }
                scheduler
                    .active
                    .lock()
                    .expect("active Agent run lock poisoned")
                    .insert(
                        agent_thread_id,
                        ActiveRun {
                            turn_id: agent_turn_id,
                            cancellation: cancellation.clone(),
                        },
                    );
                executor.execute(command, cancellation).await;
                scheduler
                    .active
                    .lock()
                    .expect("active Agent run lock poisoned")
                    .remove(&agent_thread_id);
                drop(workspace_permit);
                drop(session_permit);
                drop(global_permit);
            });
        }
    }
}

#[async_trait]
impl AgentRunScheduler for ServerAgentRunScheduler {
    async fn submit(&self, command: AgentRunCommand) -> Result<(), AgentRunSchedulerError> {
        if let AgentRunCommand::Cancel {
            agent_thread_id,
            agent_turn_id,
            ..
        } = command
        {
            if let Some(active) = self
                .active
                .lock()
                .expect("active Agent run lock poisoned")
                .get(&agent_thread_id)
                .cloned()
            {
                if active.turn_id == agent_turn_id {
                    self.turn_inbox
                        .push(agent_turn_id.as_uuid(), TurnInboxItem::Cancel);
                    active.cancellation.cancel();
                    return Ok(());
                }
            }
            self.cancelled_before_start
                .lock()
                .expect("queued cancellation lock poisoned")
                .insert(agent_turn_id);
            return Ok(());
        }
        self.sender
            .send(command)
            .map_err(|error| AgentRunSchedulerError::Unavailable(error.to_string()))
    }
}

impl AgentMailboxNotifier for ServerAgentRunScheduler {
    fn message_enqueued(&self, message: &AgentMailboxMessage) {
        let active = self
            .active
            .lock()
            .expect("active Agent run lock poisoned")
            .get(&message.to_agent_thread_id)
            .cloned();
        if let Some(active) = active {
            self.turn_inbox.push(
                active.turn_id.as_uuid(),
                TurnInboxItem::AgentMessage {
                    message: message.clone(),
                },
            );
        }
    }
}

fn command_identity(
    command: &AgentRunCommand,
) -> (CollaborationSessionId, AgentThreadId, AgentTurnId) {
    match command {
        AgentRunCommand::Start {
            session_id,
            agent_thread_id,
            agent_turn_id,
        }
        | AgentRunCommand::Resume {
            session_id,
            agent_thread_id,
            agent_turn_id,
            ..
        }
        | AgentRunCommand::Cancel {
            session_id,
            agent_thread_id,
            agent_turn_id,
        } => (*session_id, *agent_thread_id, *agent_turn_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use opentopia_core::collaboration::{
        AgentMailboxMessageId, AgentSpawnPolicy, AgentTurnRecord, CollaborationSessionPolicy,
        CreateCollaborationSession, RuntimeSnapshotSeed, SpawnAgentThread,
    };
    use opentopia_core::{BufferedTurnInbox, SessionStore, SqliteSessionStore};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::{mpsc, Semaphore};
    use tokio::time::timeout;
    use uuid::Uuid;

    struct BlockingExecutor {
        started: mpsc::UnboundedSender<(AgentThreadId, AgentTurnId)>,
        finished: mpsc::UnboundedSender<(AgentThreadId, bool)>,
        release: Arc<Semaphore>,
        active: AtomicUsize,
        max_active: AtomicUsize,
    }

    #[async_trait]
    impl AgentRunExecutor for BlockingExecutor {
        async fn execute(&self, command: AgentRunCommand, cancellation: CancellationToken) {
            let (_, agent_thread_id, agent_turn_id) = command_identity(&command);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            let _ = self.started.send((agent_thread_id, agent_turn_id));
            let cancelled = tokio::select! {
                biased;
                _ = cancellation.cancelled() => true,
                permit = self.release.acquire() => {
                    permit.expect("executor release semaphore open").forget();
                    false
                }
            };
            self.active.fetch_sub(1, Ordering::SeqCst);
            let _ = self.finished.send((agent_thread_id, cancelled));
        }
    }

    struct SchedulerFixture {
        repository: Arc<SqliteCollaborationRepository>,
        scheduler: Arc<ServerAgentRunScheduler>,
        inbox: Arc<BufferedTurnInbox>,
        session_id: CollaborationSessionId,
        root: opentopia_core::collaboration::AgentThreadRecord,
        root_turn: AgentTurnRecord,
    }

    impl SchedulerFixture {
        async fn new(session_limit: usize, global_limit: usize) -> Self {
            let store = Arc::new(SqliteSessionStore::open(":memory:").expect("scheduler store"));
            let workspace = PathBuf::from("C:/scheduler-fixture");
            let user_thread = store
                .create_thread(None, workspace.clone())
                .expect("scheduler user thread");
            let repository =
                Arc::new(SqliteCollaborationRepository::new(store).expect("scheduler repository"));
            let (session, root, root_turn) = repository
                .create_session(CreateCollaborationSession {
                    user_task_id: user_thread.id,
                    root_turn_id: AgentTurnId::new(),
                    root_task_message: "scheduler root".to_string(),
                    root_agent_type: "default".to_string(),
                    root_runtime_snapshot: RuntimeSnapshotSeed::new(
                        None,
                        json!({
                            "workspaceMode": "shared_read_only",
                            "workspaceRoot": workspace,
                            "provider": {},
                            "permissionMode": "read_only",
                            "sandbox": {},
                            "capabilityProjection": {},
                        }),
                    ),
                    session_policy: CollaborationSessionPolicy {
                        max_agents: 16,
                        max_active_runs: session_limit,
                        max_depth: 1,
                    },
                    root_spawn_policy: AgentSpawnPolicy::allows_children(1, 12),
                })
                .await
                .expect("scheduler collaboration session");
            let inbox = Arc::new(BufferedTurnInbox::default());
            let scheduler =
                ServerAgentRunScheduler::new(repository.clone(), inbox.clone(), global_limit);
            Self {
                repository,
                scheduler,
                inbox,
                session_id: session.id,
                root,
                root_turn,
            }
        }

        async fn child(&self, name: &str, workspace_mode: &str) -> AgentTurnRecord {
            let (_, turn) = self
                .repository
                .spawn_agent(SpawnAgentThread {
                    parent_agent_thread_id: self.root.id,
                    requested_by_turn_id: self.root_turn.id,
                    task_name: name.to_string(),
                    agent_type: "default".to_string(),
                    task_message: format!("run {name}"),
                    runtime_snapshot: RuntimeSnapshotSeed::new(
                        Some(self.root.runtime_snapshot_id),
                        json!({
                            "workspaceMode": workspace_mode,
                            "workspaceRoot": "C:/scheduler-fixture",
                            "provider": {},
                            "permissionMode": "read_only",
                            "sandbox": {},
                            "capabilityProjection": {},
                        }),
                    ),
                    spawn_policy: AgentSpawnPolicy::disabled(1),
                })
                .await
                .expect("scheduler child");
            turn
        }

        fn start_command(&self, turn: &AgentTurnRecord) -> AgentRunCommand {
            AgentRunCommand::Start {
                session_id: self.session_id,
                agent_thread_id: turn.agent_thread_id,
                agent_turn_id: turn.id,
            }
        }

        fn cancel_command(&self, turn: &AgentTurnRecord) -> AgentRunCommand {
            AgentRunCommand::Cancel {
                session_id: self.session_id,
                agent_thread_id: turn.agent_thread_id,
                agent_turn_id: turn.id,
            }
        }
    }

    fn blocking_executor() -> (
        Arc<BlockingExecutor>,
        mpsc::UnboundedReceiver<(AgentThreadId, AgentTurnId)>,
        mpsc::UnboundedReceiver<(AgentThreadId, bool)>,
    ) {
        let (started, started_rx) = mpsc::unbounded_channel();
        let (finished, finished_rx) = mpsc::unbounded_channel();
        (
            Arc::new(BlockingExecutor {
                started,
                finished,
                release: Arc::new(Semaphore::new(0)),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
            }),
            started_rx,
            finished_rx,
        )
    }

    async fn receive<T>(receiver: &mut mpsc::UnboundedReceiver<T>) -> T {
        timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("scheduler observation arrived")
            .expect("scheduler observation channel open")
    }

    #[tokio::test]
    async fn scheduler_enforces_session_concurrency_without_losing_queued_runs() {
        let fixture = SchedulerFixture::new(2, 4).await;
        let mut turns = Vec::new();
        for index in 0..8 {
            turns.push(
                fixture
                    .child(&format!("burst_{index}"), "shared_read_only")
                    .await,
            );
        }
        let (executor, mut started, mut finished) = blocking_executor();
        fixture.scheduler.start(executor.clone());
        for turn in &turns {
            fixture
                .scheduler
                .submit(fixture.start_command(turn))
                .await
                .expect("submit run");
        }

        let _ = receive(&mut started).await;
        let _ = receive(&mut started).await;
        assert!(
            timeout(Duration::from_millis(100), started.recv())
                .await
                .is_err(),
            "additional runs must remain queued behind the per-session limit"
        );
        assert_eq!(executor.max_active.load(Ordering::SeqCst), 2);

        for batch in 0..4 {
            executor.release.add_permits(2);
            let _ = receive(&mut finished).await;
            let _ = receive(&mut finished).await;
            if batch < 3 {
                let _ = receive(&mut started).await;
                let _ = receive(&mut started).await;
                assert_eq!(executor.max_active.load(Ordering::SeqCst), 2);
            }
        }
    }

    #[tokio::test]
    async fn scheduler_serializes_shared_coordinated_writers_per_workspace() {
        let fixture = SchedulerFixture::new(3, 3).await;
        let first = fixture.child("writer_one", "shared_coordinated").await;
        let second = fixture.child("writer_two", "shared_coordinated").await;
        let (executor, mut started, mut finished) = blocking_executor();
        fixture.scheduler.start(executor.clone());
        fixture
            .scheduler
            .submit(fixture.start_command(&first))
            .await
            .unwrap();
        fixture
            .scheduler
            .submit(fixture.start_command(&second))
            .await
            .unwrap();

        let _ = receive(&mut started).await;
        assert!(timeout(Duration::from_millis(100), started.recv())
            .await
            .is_err());
        assert_eq!(executor.max_active.load(Ordering::SeqCst), 1);
        executor.release.add_permits(1);
        let _ = receive(&mut finished).await;
        let _ = receive(&mut started).await;
        executor.release.add_permits(1);
        let _ = receive(&mut finished).await;
    }

    #[tokio::test]
    async fn scheduler_cancels_both_queued_and_running_runs() {
        let queued_fixture = SchedulerFixture::new(1, 1).await;
        let queued_turn = queued_fixture.child("queued", "shared_read_only").await;
        queued_fixture
            .scheduler
            .submit(queued_fixture.start_command(&queued_turn))
            .await
            .unwrap();
        queued_fixture
            .scheduler
            .submit(queued_fixture.cancel_command(&queued_turn))
            .await
            .unwrap();
        let (queued_executor, mut queued_started, mut queued_finished) = blocking_executor();
        queued_fixture.scheduler.start(queued_executor);
        let _ = receive(&mut queued_started).await;
        assert_eq!(
            receive(&mut queued_finished).await,
            (queued_turn.agent_thread_id, true)
        );

        let running_fixture = SchedulerFixture::new(1, 1).await;
        let running_turn = running_fixture.child("running", "shared_read_only").await;
        let (running_executor, mut running_started, mut running_finished) = blocking_executor();
        running_fixture.scheduler.start(running_executor);
        running_fixture
            .scheduler
            .submit(running_fixture.start_command(&running_turn))
            .await
            .unwrap();
        let _ = receive(&mut running_started).await;
        running_fixture
            .scheduler
            .submit(running_fixture.cancel_command(&running_turn))
            .await
            .unwrap();
        assert_eq!(
            receive(&mut running_finished).await,
            (running_turn.agent_thread_id, true)
        );
        assert!(running_fixture
            .inbox
            .drain(running_turn.id.as_uuid())
            .iter()
            .any(|item| matches!(item, TurnInboxItem::Cancel)));
    }

    #[tokio::test]
    async fn scheduler_delivers_live_mailbox_messages_to_the_active_turn() {
        let fixture = SchedulerFixture::new(1, 1).await;
        let turn = fixture.child("messaged", "shared_read_only").await;
        let (executor, mut started, mut finished) = blocking_executor();
        fixture.scheduler.start(executor.clone());
        fixture
            .scheduler
            .submit(fixture.start_command(&turn))
            .await
            .unwrap();
        let _ = receive(&mut started).await;
        let message = AgentMailboxMessage {
            id: AgentMailboxMessageId::new(),
            session_id: fixture.session_id,
            sequence: 1,
            from_agent_thread_id: fixture.root.id,
            to_agent_thread_id: turn.agent_thread_id,
            kind: opentopia_core::collaboration::AgentMailboxMessageKind::Message,
            payload: json!({ "text": "live context" }),
            causation_id: Some(Uuid::new_v4()),
            created_at: Utc::now(),
            delivered_at: None,
            acknowledged_at: None,
        };
        fixture.scheduler.message_enqueued(&message);
        let delivered = fixture.inbox.drain(turn.id.as_uuid());
        assert!(delivered.iter().any(|item| matches!(
            item,
            TurnInboxItem::AgentMessage { message: delivered } if delivered.id == message.id
        )));
        executor.release.add_permits(1);
        let _ = receive(&mut finished).await;
    }
}
