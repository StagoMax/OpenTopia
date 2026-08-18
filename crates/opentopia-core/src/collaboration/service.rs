use super::{
    AgentMailbox, AgentMailboxError, AgentMailboxMessage, AgentMailboxMessageKind,
    AgentMailboxNotifier, AgentRunCommand, AgentRunScheduler, AgentRunSchedulerError,
    AgentThreadId, AgentThreadRecord, AgentTurnId, AgentTurnRecord, CollaborationDomainError,
    CollaborationRegistry, EnqueueAgentMessage, FollowupAgentTurn, SpawnAgentThread,
};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SpawnAgentOutcome {
    pub agent: AgentThreadRecord,
    pub turn: AgentTurnRecord,
}

#[derive(Debug, Error)]
pub enum AgentCollaborationRuntimeError {
    #[error(transparent)]
    Domain(#[from] CollaborationDomainError),
    #[error(transparent)]
    Mailbox(#[from] AgentMailboxError),
    #[error("agent run command for {agent_turn_id} could not be submitted: {source}")]
    RunSubmission {
        agent_turn_id: AgentTurnId,
        #[source]
        source: AgentRunSchedulerError,
    },
}

/// Tool-facing collaboration service.
///
/// This service owns no execution loop. It persists collaboration state through
/// the registry and submits whole-run commands through a separate scheduler
/// port. A failed scheduler submission intentionally leaves the durable Turn in
/// `queued`, allowing recovery to retry without recreating Agent identity.
#[derive(Clone)]
pub struct AgentCollaborationRuntime {
    registry: Arc<dyn CollaborationRegistry>,
    run_scheduler: Arc<dyn AgentRunScheduler>,
    mailbox: Arc<dyn AgentMailbox>,
    mailbox_notifier: Arc<dyn AgentMailboxNotifier>,
}

impl AgentCollaborationRuntime {
    pub fn new(
        registry: Arc<dyn CollaborationRegistry>,
        run_scheduler: Arc<dyn AgentRunScheduler>,
        mailbox: Arc<dyn AgentMailbox>,
    ) -> Self {
        Self {
            registry,
            run_scheduler,
            mailbox,
            mailbox_notifier: Arc::new(super::NoopAgentMailboxNotifier),
        }
    }

    pub fn with_mailbox_notifier(mut self, notifier: Arc<dyn AgentMailboxNotifier>) -> Self {
        self.mailbox_notifier = notifier;
        self
    }

    pub fn registry(&self) -> &Arc<dyn CollaborationRegistry> {
        &self.registry
    }

    pub fn mailbox(&self) -> &Arc<dyn AgentMailbox> {
        &self.mailbox
    }

    pub async fn spawn_agent(
        &self,
        request: SpawnAgentThread,
    ) -> Result<SpawnAgentOutcome, AgentCollaborationRuntimeError> {
        let (agent, turn) = self.registry.spawn_agent(request).await?;
        self.submit_start(&agent, &turn).await?;
        Ok(SpawnAgentOutcome { agent, turn })
    }

    pub async fn followup_task(
        &self,
        request: FollowupAgentTurn,
    ) -> Result<AgentTurnRecord, AgentCollaborationRuntimeError> {
        let turn = self.registry.create_followup_turn(request).await?;
        let agent = self.registry.get_thread(turn.agent_thread_id).await?;
        self.submit_start(&agent, &turn).await?;
        Ok(turn)
    }

    pub async fn send_message(
        &self,
        caller: AgentThreadId,
        target: AgentThreadId,
        kind: AgentMailboxMessageKind,
        payload: Value,
        causation_id: Option<uuid::Uuid>,
    ) -> Result<AgentMailboxMessage, AgentCollaborationRuntimeError> {
        let caller = self.registry.get_thread(caller).await?;
        let target = self.registry.get_thread(target).await?;
        if caller.session_id != target.session_id {
            return Err(CollaborationDomainError::AgentThreadNotFound(target.id).into());
        }
        let message = self
            .mailbox
            .enqueue(EnqueueAgentMessage {
                session_id: caller.session_id,
                from_agent_thread_id: caller.id,
                to_agent_thread_id: target.id,
                kind,
                payload,
                causation_id,
            })
            .await?;
        self.mailbox_notifier.message_enqueued(&message);
        Ok(message)
    }

    /// Requests cancellation at the whole-Agent-Run boundary. Descendant
    /// cancellation policy belongs to the scheduler implementation; this
    /// service never reaches into Agent Core's internal loop.
    pub async fn interrupt_agent(
        &self,
        target: &AgentThreadRecord,
    ) -> Result<Option<AgentTurnRecord>, AgentCollaborationRuntimeError> {
        let Some(turn) = self.registry.latest_turn(target.id).await? else {
            return Ok(None);
        };
        if turn.status.is_terminal() {
            return Ok(Some(turn));
        }
        let mut subtree = self
            .registry
            .list_threads(target.session_id)
            .await?
            .into_iter()
            .filter(|agent| agent.id == target.id || agent.path.is_descendant_of(&target.path))
            .collect::<Vec<_>>();
        subtree.sort_by_key(|agent| std::cmp::Reverse(agent.path.depth()));
        for agent in subtree {
            let Some(agent_turn) = self.registry.latest_turn(agent.id).await? else {
                continue;
            };
            if agent_turn.status.is_terminal() {
                continue;
            }
            self.run_scheduler
                .submit(AgentRunCommand::Cancel {
                    session_id: agent.session_id,
                    agent_thread_id: agent.id,
                    agent_turn_id: agent_turn.id,
                })
                .await
                .map_err(|source| AgentCollaborationRuntimeError::RunSubmission {
                    agent_turn_id: agent_turn.id,
                    source,
                })?;
        }
        Ok(Some(turn))
    }

    async fn submit_start(
        &self,
        agent: &AgentThreadRecord,
        turn: &AgentTurnRecord,
    ) -> Result<(), AgentCollaborationRuntimeError> {
        self.run_scheduler
            .submit(AgentRunCommand::Start {
                session_id: agent.session_id,
                agent_thread_id: agent.id,
                agent_turn_id: turn.id,
            })
            .await
            .map_err(|source| AgentCollaborationRuntimeError::RunSubmission {
                agent_turn_id: turn.id,
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{test_runtime_snapshot, RuntimeWorkspaceModeV1};
    use crate::collaboration::{
        AgentSpawnPolicy, AgentTurnStatus, CollaborationRegistry, CollaborationSessionPolicy,
        CreateCollaborationSession, InMemoryAgentMailbox, InMemoryCollaborationRegistry,
        RuntimeSnapshotSeed,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct RecordingRunScheduler {
        commands: Mutex<Vec<AgentRunCommand>>,
        fail: bool,
    }

    #[async_trait]
    impl AgentRunScheduler for RecordingRunScheduler {
        async fn submit(&self, command: AgentRunCommand) -> Result<(), AgentRunSchedulerError> {
            if self.fail {
                return Err(AgentRunSchedulerError::Unavailable("offline".to_string()));
            }
            self.commands.lock().unwrap().push(command);
            Ok(())
        }
    }

    async fn runtime(
        scheduler: Arc<RecordingRunScheduler>,
    ) -> (
        AgentCollaborationRuntime,
        Arc<InMemoryCollaborationRegistry>,
        AgentThreadRecord,
        AgentTurnRecord,
    ) {
        let registry = Arc::new(InMemoryCollaborationRegistry::new());
        let (_, root, root_turn) = registry
            .create_session(CreateCollaborationSession {
                user_task_id: Uuid::new_v4(),
                root_turn_id: super::AgentTurnId::new(),
                root_task_message: "root".to_string(),
                root_agent_type: "default".to_string(),
                root_runtime_snapshot: RuntimeSnapshotSeed::new(
                    None,
                    test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                session_policy: CollaborationSessionPolicy {
                    max_agents: 8,
                    max_active_runs: 4,
                    max_depth: 1,
                },
                root_spawn_policy: AgentSpawnPolicy::allows_children(1, 4),
            })
            .await
            .unwrap();
        (
            AgentCollaborationRuntime::new(
                registry.clone(),
                scheduler,
                Arc::new(InMemoryAgentMailbox::new()),
            ),
            registry,
            root,
            root_turn,
        )
    }

    #[tokio::test]
    async fn spawn_submits_whole_run_without_driving_agent_core() {
        let scheduler = Arc::new(RecordingRunScheduler::default());
        let (runtime, _, root, root_turn) = runtime(scheduler.clone()).await;
        let outcome = runtime
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "research".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "inspect".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(1),
            })
            .await
            .unwrap();

        assert_eq!(outcome.turn.status, AgentTurnStatus::Queued);
        assert_eq!(
            scheduler.commands.lock().unwrap().as_slice(),
            &[AgentRunCommand::Start {
                session_id: outcome.agent.session_id,
                agent_thread_id: outcome.agent.id,
                agent_turn_id: outcome.turn.id,
            }]
        );
    }

    #[tokio::test]
    async fn failed_submission_preserves_queued_identity_for_recovery() {
        let scheduler = Arc::new(RecordingRunScheduler {
            commands: Mutex::new(Vec::new()),
            fail: true,
        });
        let (runtime, registry, root, root_turn) = runtime(scheduler).await;
        let error = runtime
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "research".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "inspect".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(1),
            })
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AgentCollaborationRuntimeError::RunSubmission { .. }
        ));

        let child = registry
            .resolve_path(root.session_id, &root.path.child("research").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            registry
                .latest_turn(child.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            AgentTurnStatus::Queued
        );
    }

    #[tokio::test]
    async fn messages_use_the_shared_mailbox_without_touching_run_scheduler() {
        let scheduler = Arc::new(RecordingRunScheduler::default());
        let (runtime, _, root, root_turn) = runtime(scheduler.clone()).await;
        let child = runtime
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "research".to_string(),
                agent_type: "explorer".to_string(),
                task_message: "inspect".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(1),
            })
            .await
            .unwrap()
            .agent;
        let commands_before_message = scheduler.commands.lock().unwrap().len();

        let message = runtime
            .send_message(
                root.id,
                child.id,
                AgentMailboxMessageKind::Message,
                serde_json::json!({ "text": "check this too" }),
                Some(Uuid::new_v4()),
            )
            .await
            .unwrap();

        assert_eq!(message.from_agent_thread_id, root.id);
        assert_eq!(message.to_agent_thread_id, child.id);
        assert_eq!(
            runtime
                .mailbox()
                .snapshot(root.session_id, child.id, None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            scheduler.commands.lock().unwrap().len(),
            commands_before_message,
            "sending a message must not submit another Agent run"
        );
    }
}
