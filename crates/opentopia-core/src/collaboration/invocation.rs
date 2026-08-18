use super::{
    ActivityQuery, AgentActivitySource, AgentActivitySourceError, AgentActivityWindow,
    AgentAvailability, AgentCollaborationRuntime, AgentCollaborationRuntimeError,
    AgentMailboxMessage, AgentMailboxMessageKind, AgentPath, AgentRuntimeSnapshotRecord,
    AgentSpawnPolicy, AgentThreadId, AgentThreadRecord, AgentTurnId, AgentTurnRecord,
    CollaborationDomainError, CollaborationSessionId, FollowupAgentTurn, RuntimeSnapshotId,
    RuntimeSnapshotSeed, SpawnAgentOutcome, SpawnAgentThread,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

/// Immutable identity captured at the Agent Turn -> Tool Runtime boundary.
///
/// Tool arguments never contain these fields, so a model cannot impersonate a
/// different Session, AgentThread, Turn, or Runtime Snapshot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInvocationIdentity {
    pub session_id: CollaborationSessionId,
    pub agent_thread_id: AgentThreadId,
    pub agent_turn_id: AgentTurnId,
    pub runtime_snapshot_id: RuntimeSnapshotId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ForkTurns {
    None,
    All,
    Count(usize),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkspaceMode {
    Auto,
    SharedReadOnly,
    SharedCoordinated,
    IsolatedWorktree,
}

#[derive(Debug, Clone)]
pub struct ChildRuntimeSnapshotRequest {
    pub agent_type: String,
    pub fork_turns: ForkTurns,
    pub workspace_mode: AgentWorkspaceMode,
    pub allow_child_spawns: bool,
}

#[derive(Debug, Clone)]
pub struct DerivedChildRuntime {
    pub runtime_snapshot: RuntimeSnapshotSeed,
    pub spawn_policy: AgentSpawnPolicy,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeSnapshotDerivationError {
    #[error("child runtime snapshot request was rejected: {0}")]
    Rejected(String),
    #[error("child runtime snapshot derivation failed: {0}")]
    Unavailable(String),
}

/// Derives a child capability snapshot from the caller's already-frozen
/// snapshot. Implementations live at the application composition boundary,
/// where model, tools, plugins, sandbox, and workspace leases are available.
/// They must only attenuate capabilities.
#[async_trait]
pub trait RuntimeSnapshotDeriver: Send + Sync {
    async fn derive_child(
        &self,
        parent: &AgentRuntimeSnapshotRecord,
        request: ChildRuntimeSnapshotRequest,
    ) -> Result<DerivedChildRuntime, RuntimeSnapshotDerivationError>;
}

#[derive(Debug, Clone)]
pub struct SpawnChildAgentRequest {
    pub task_name: String,
    pub message: String,
    pub agent_type: String,
    pub fork_turns: ForkTurns,
    pub workspace_mode: AgentWorkspaceMode,
    pub allow_child_spawns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentListItem {
    pub agent: AgentThreadRecord,
    pub latest_turn: Option<AgentTurnRecord>,
    pub availability: AgentAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<AgentActivityWindow>,
}

#[derive(Debug, Clone)]
pub struct WaitAgentRequest {
    pub target: Option<String>,
    pub after_cursor: Option<i64>,
    pub timeout: Duration,
    pub reasoning_tail_chars: usize,
    pub tool_result_chars: usize,
    pub event_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WaitAgentOutcome {
    pub timed_out: bool,
    pub agent: AgentThreadRecord,
    pub turn: Option<AgentTurnRecord>,
    pub availability: AgentAvailability,
    pub activity: Option<AgentActivityWindow>,
    pub messages: Vec<AgentMailboxMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCompletionSnapshot {
    pub active_descendants: Vec<AgentListItem>,
    pub pending_messages: Vec<AgentMailboxMessage>,
}

#[derive(Debug, Error)]
pub enum AgentCollaborationInvocationError {
    #[error(transparent)]
    Domain(#[from] CollaborationDomainError),
    #[error(transparent)]
    Runtime(#[from] AgentCollaborationRuntimeError),
    #[error(transparent)]
    Snapshot(#[from] RuntimeSnapshotDerivationError),
    #[error(transparent)]
    Activity(#[from] AgentActivitySourceError),
    #[error("agent target `{0}` was not found in the current collaboration session")]
    TargetNotFound(String),
    #[error(
        "the current tool invocation identity is inconsistent with persisted collaboration state"
    )]
    InvalidInvocationIdentity,
}

/// A tool-facing, caller-bound collaboration capability.
///
/// This is intentionally one field in ToolInvocationContext. It replaces the
/// old loose combination of scheduler, thread UUID, parent Turn UUID, depth,
/// and path, which could drift apart during recursive execution.
#[derive(Clone)]
pub struct AgentCollaborationInvocation {
    runtime: AgentCollaborationRuntime,
    activity: Arc<dyn AgentActivitySource>,
    snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver>,
    identity: AgentInvocationIdentity,
}

impl AgentCollaborationInvocation {
    pub fn new(
        runtime: AgentCollaborationRuntime,
        activity: Arc<dyn AgentActivitySource>,
        snapshot_deriver: Arc<dyn RuntimeSnapshotDeriver>,
        identity: AgentInvocationIdentity,
    ) -> Self {
        Self {
            runtime,
            activity,
            snapshot_deriver,
            identity,
        }
    }

    pub fn identity(&self) -> AgentInvocationIdentity {
        self.identity
    }

    pub async fn pending_messages(
        &self,
        limit: usize,
    ) -> Result<Vec<AgentMailboxMessage>, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        self.runtime
            .mailbox()
            .snapshot(caller.session_id, caller.id, None, limit)
            .await
            .map_err(AgentCollaborationRuntimeError::from)
            .map_err(Into::into)
    }

    pub async fn completion_snapshot(
        &self,
    ) -> Result<AgentCompletionSnapshot, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let mut active_descendants = Vec::new();
        for agent in self
            .runtime
            .registry()
            .list_threads(caller.session_id)
            .await?
            .into_iter()
            .filter(|agent| agent.path.is_descendant_of(&caller.path))
        {
            let latest_turn = self.runtime.registry().latest_turn(agent.id).await?;
            if latest_turn
                .as_ref()
                .is_some_and(|turn| !turn.status.is_terminal())
            {
                active_descendants.push(AgentListItem {
                    availability: AgentAvailability::derive(&agent, latest_turn.as_ref()),
                    agent,
                    latest_turn,
                    activity: None,
                });
            }
        }
        let pending_messages = self
            .runtime
            .mailbox()
            .snapshot(caller.session_id, caller.id, None, 256)
            .await
            .map_err(AgentCollaborationRuntimeError::from)?;
        Ok(AgentCompletionSnapshot {
            active_descendants,
            pending_messages,
        })
    }

    /// This is invoked only after the provider round carrying the synthetic
    /// mailbox Call/Result pair has completed. A crash before the durable ack
    /// simply causes an idempotent redelivery on the next invocation.
    pub async fn acknowledge_messages(
        &self,
        messages: &[AgentMailboxMessage],
    ) -> Result<(), AgentCollaborationInvocationError> {
        if messages.is_empty() {
            return Ok(());
        }
        let target = self.identity.agent_thread_id;
        let ids = messages
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>();
        self.runtime
            .mailbox()
            .acknowledge(target, &ids)
            .await
            .map_err(AgentCollaborationRuntimeError::from)?;
        Ok(())
    }

    pub async fn spawn_agent(
        &self,
        request: SpawnChildAgentRequest,
    ) -> Result<SpawnAgentOutcome, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let parent_snapshot = self
            .runtime
            .registry()
            .get_runtime_snapshot(caller.runtime_snapshot_id)
            .await?;
        let derived = self
            .snapshot_deriver
            .derive_child(
                &parent_snapshot,
                ChildRuntimeSnapshotRequest {
                    agent_type: request.agent_type.clone(),
                    fork_turns: request.fork_turns,
                    workspace_mode: request.workspace_mode,
                    allow_child_spawns: request.allow_child_spawns,
                },
            )
            .await?;
        Ok(self
            .runtime
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: caller.id,
                requested_by_turn_id: self.identity.agent_turn_id,
                task_name: request.task_name,
                agent_type: request.agent_type,
                task_message: request.message,
                runtime_snapshot: derived.runtime_snapshot,
                spawn_policy: derived.spawn_policy,
            })
            .await?)
    }

    pub async fn send_message(
        &self,
        target: &str,
        message: String,
        causation_id: Option<Uuid>,
    ) -> Result<AgentMailboxMessage, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let target = self.resolve_target(&caller, target).await?;
        Ok(self
            .runtime
            .send_message(
                caller.id,
                target.id,
                AgentMailboxMessageKind::Message,
                json!({ "text": message }),
                causation_id,
            )
            .await?)
    }

    pub async fn followup_task(
        &self,
        target: &str,
        message: String,
    ) -> Result<AgentTurnRecord, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let target = self.resolve_target(&caller, target).await?;
        Ok(self
            .runtime
            .followup_task(FollowupAgentTurn {
                requested_by_agent_thread_id: caller.id,
                requested_by_turn_id: self.identity.agent_turn_id,
                target_agent_thread_id: target.id,
                task_message: message,
            })
            .await?)
    }

    pub async fn interrupt_agent(
        &self,
        target: &str,
    ) -> Result<Option<AgentTurnRecord>, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let target = self.resolve_target(&caller, target).await?;
        if !target.path.is_descendant_of(&caller.path) {
            return Err(CollaborationDomainError::LifecyclePermissionDenied {
                caller: caller.path,
                target: target.path,
            }
            .into());
        }
        Ok(self.runtime.interrupt_agent(&target).await?)
    }

    pub async fn list_agents(
        &self,
        path_prefix: Option<&str>,
    ) -> Result<Vec<AgentListItem>, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let prefix = path_prefix
            .map(AgentPath::parse)
            .transpose()?
            .unwrap_or_else(AgentPath::root);
        let threads = self
            .runtime
            .registry()
            .list_threads(caller.session_id)
            .await?;
        let mut items = Vec::new();
        for agent in threads
            .into_iter()
            .filter(|agent| agent.path == prefix || agent.path.is_descendant_of(&prefix))
        {
            let latest_turn = self.runtime.registry().latest_turn(agent.id).await?;
            let availability = AgentAvailability::derive(&agent, latest_turn.as_ref());
            let activity = match latest_turn.as_ref() {
                Some(turn) => Some(
                    self.activity
                        .read_activity(
                            agent.id,
                            turn.id,
                            turn.status,
                            ActivityQuery {
                                reasoning_tail_chars: 512,
                                tool_result_chars: 1_000,
                                event_limit: 4,
                                ..ActivityQuery::default()
                            },
                        )
                        .await?,
                ),
                None => None,
            };
            items.push(AgentListItem {
                agent,
                latest_turn,
                availability,
                activity,
            });
        }
        Ok(items)
    }

    pub async fn wait_agent(
        &self,
        request: WaitAgentRequest,
    ) -> Result<WaitAgentOutcome, AgentCollaborationInvocationError> {
        let caller = self.validated_caller().await?;
        let requested_target = match request.target.as_deref() {
            Some(target) => Some(self.resolve_target(&caller, target).await?),
            None => None,
        };
        let changed_target = if request.timeout.is_zero() {
            requested_target.as_ref().map(|agent| agent.id)
        } else {
            self.activity
                .wait_for_change(
                    caller.session_id,
                    requested_target.as_ref().map(|agent| agent.id),
                    request.after_cursor,
                    request.timeout,
                )
                .await?
        };
        let timed_out = !request.timeout.is_zero() && changed_target.is_none();
        let agent = match changed_target {
            Some(id) => self.runtime.registry().get_thread(id).await?,
            None => requested_target.unwrap_or(caller.clone()),
        };
        let turn = self.runtime.registry().latest_turn(agent.id).await?;
        let availability = AgentAvailability::derive(&agent, turn.as_ref());
        let activity = match turn.as_ref() {
            Some(turn) => Some(
                self.activity
                    .read_activity(
                        agent.id,
                        turn.id,
                        turn.status,
                        ActivityQuery {
                            after_cursor: request.after_cursor,
                            reasoning_tail_chars: request.reasoning_tail_chars,
                            tool_result_chars: request.tool_result_chars,
                            event_limit: request.event_limit,
                        },
                    )
                    .await?,
            ),
            None => None,
        };
        let messages = self
            .runtime
            .mailbox()
            .snapshot(caller.session_id, caller.id, None, 64)
            .await
            .map_err(AgentCollaborationRuntimeError::from)?;
        Ok(WaitAgentOutcome {
            timed_out,
            agent,
            turn,
            availability,
            activity,
            messages,
        })
    }

    async fn validated_caller(
        &self,
    ) -> Result<AgentThreadRecord, AgentCollaborationInvocationError> {
        let caller = self
            .runtime
            .registry()
            .get_thread(self.identity.agent_thread_id)
            .await?;
        let turn = self
            .runtime
            .registry()
            .get_turn(self.identity.agent_turn_id)
            .await?;
        if caller.session_id != self.identity.session_id
            || caller.runtime_snapshot_id != self.identity.runtime_snapshot_id
            || turn.session_id != self.identity.session_id
            || turn.agent_thread_id != caller.id
        {
            return Err(AgentCollaborationInvocationError::InvalidInvocationIdentity);
        }
        Ok(caller)
    }

    async fn resolve_target(
        &self,
        caller: &AgentThreadRecord,
        target: &str,
    ) -> Result<AgentThreadRecord, AgentCollaborationInvocationError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(AgentCollaborationInvocationError::TargetNotFound(
                target.to_string(),
            ));
        }
        if let Ok(id) = Uuid::parse_str(target) {
            let id = AgentThreadId::from_uuid(id);
            return self
                .runtime
                .registry()
                .get_thread(id)
                .await
                .ok()
                .filter(|agent| agent.session_id == caller.session_id)
                .ok_or_else(|| AgentCollaborationInvocationError::TargetNotFound(target.into()));
        }
        let path = if target.starts_with('/') {
            AgentPath::parse(target)?
        } else {
            caller.path.child(target)?
        };
        self.runtime
            .registry()
            .resolve_path(caller.session_id, &path)
            .await?
            .ok_or_else(|| AgentCollaborationInvocationError::TargetNotFound(target.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collaboration::{
        test_runtime_snapshot, AgentActivitySource, AgentRunCommand, AgentRunScheduler,
        AgentRunSchedulerError, AgentTurnStatus, CollaborationRegistry, CollaborationSessionPolicy,
        CreateCollaborationSession, InMemoryAgentMailbox, InMemoryCollaborationRegistry,
        RuntimeWorkspaceModeV1,
    };
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingRunScheduler {
        commands: Mutex<Vec<AgentRunCommand>>,
    }

    #[async_trait]
    impl AgentRunScheduler for RecordingRunScheduler {
        async fn submit(&self, command: AgentRunCommand) -> Result<(), AgentRunSchedulerError> {
            self.commands.lock().unwrap().push(command);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FixtureActivitySource;

    #[async_trait]
    impl AgentActivitySource for FixtureActivitySource {
        async fn read_activity(
            &self,
            agent_thread_id: AgentThreadId,
            agent_turn_id: AgentTurnId,
            turn_status: AgentTurnStatus,
            query: ActivityQuery,
        ) -> Result<AgentActivityWindow, AgentActivitySourceError> {
            Ok(AgentActivityWindow {
                agent_thread_id,
                agent_turn_id,
                turn_status,
                model_round: Some(3),
                cursor: query.after_cursor.unwrap_or(6) + 1,
                reasoning_tail: Some("inspecting the current tool call".to_string()),
                recent_events: Vec::new(),
                recent_tool_results: Vec::new(),
            })
        }

        async fn wait_for_change(
            &self,
            _session_id: CollaborationSessionId,
            target: Option<AgentThreadId>,
            _after_cursor: Option<i64>,
            _timeout: Duration,
        ) -> Result<Option<AgentThreadId>, AgentActivitySourceError> {
            Ok(target)
        }
    }

    #[derive(Default)]
    struct FixtureSnapshotDeriver;

    #[async_trait]
    impl RuntimeSnapshotDeriver for FixtureSnapshotDeriver {
        async fn derive_child(
            &self,
            parent: &AgentRuntimeSnapshotRecord,
            request: ChildRuntimeSnapshotRequest,
        ) -> Result<DerivedChildRuntime, RuntimeSnapshotDerivationError> {
            Ok(DerivedChildRuntime {
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(parent.id),
                    test_runtime_snapshot(
                        &request.agent_type,
                        RuntimeWorkspaceModeV1::SharedReadOnly,
                    ),
                ),
                spawn_policy: if request.allow_child_spawns {
                    AgentSpawnPolicy::allows_children(2, 2)
                } else {
                    AgentSpawnPolicy::disabled(2)
                },
            })
        }
    }

    struct Fixture {
        runtime: AgentCollaborationRuntime,
        registry: Arc<InMemoryCollaborationRegistry>,
        scheduler: Arc<RecordingRunScheduler>,
        activity: Arc<FixtureActivitySource>,
        deriver: Arc<FixtureSnapshotDeriver>,
        root: AgentThreadRecord,
        root_turn: AgentTurnRecord,
    }

    impl Fixture {
        async fn new() -> Self {
            let registry = Arc::new(InMemoryCollaborationRegistry::new());
            let mailbox = Arc::new(InMemoryAgentMailbox::new());
            let scheduler = Arc::new(RecordingRunScheduler::default());
            let activity = Arc::new(FixtureActivitySource);
            let deriver = Arc::new(FixtureSnapshotDeriver);
            let (_, root, root_turn) = registry
                .create_session(CreateCollaborationSession {
                    user_task_id: Uuid::new_v4(),
                    root_turn_id: AgentTurnId::new(),
                    root_task_message: "root task".to_string(),
                    root_agent_type: "default".to_string(),
                    root_runtime_snapshot: RuntimeSnapshotSeed::new(
                        None,
                        test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated),
                    ),
                    session_policy: CollaborationSessionPolicy {
                        max_agents: 8,
                        max_active_runs: 4,
                        max_depth: 2,
                    },
                    root_spawn_policy: AgentSpawnPolicy::allows_children(2, 4),
                })
                .await
                .unwrap();
            let runtime =
                AgentCollaborationRuntime::new(registry.clone(), scheduler.clone(), mailbox);
            Self {
                runtime,
                registry,
                scheduler,
                activity,
                deriver,
                root,
                root_turn,
            }
        }

        fn invocation(
            &self,
            agent: &AgentThreadRecord,
            turn: &AgentTurnRecord,
        ) -> AgentCollaborationInvocation {
            AgentCollaborationInvocation::new(
                self.runtime.clone(),
                self.activity.clone(),
                self.deriver.clone(),
                AgentInvocationIdentity {
                    session_id: agent.session_id,
                    agent_thread_id: agent.id,
                    agent_turn_id: turn.id,
                    runtime_snapshot_id: agent.runtime_snapshot_id,
                },
            )
        }
    }

    fn spawn_request(task_name: &str, allow_child_spawns: bool) -> SpawnChildAgentRequest {
        SpawnChildAgentRequest {
            task_name: task_name.to_string(),
            message: format!("work on {task_name}"),
            agent_type: "default".to_string(),
            fork_turns: ForkTurns::None,
            workspace_mode: AgentWorkspaceMode::SharedReadOnly,
            allow_child_spawns,
        }
    }

    #[tokio::test]
    async fn one_bound_capability_supports_recursive_spawn_and_all_control_operations() {
        let fixture = Fixture::new().await;
        let root = fixture.invocation(&fixture.root, &fixture.root_turn);
        let child = root
            .spawn_agent(spawn_request("research", true))
            .await
            .unwrap();
        let child_invocation = fixture.invocation(&child.agent, &child.turn);
        let grandchild = child_invocation
            .spawn_agent(spawn_request("review", false))
            .await
            .unwrap();

        assert_eq!(grandchild.agent.path.as_str(), "/root/research/review");
        let child_snapshot = fixture
            .registry
            .get_runtime_snapshot(child.agent.runtime_snapshot_id)
            .await
            .unwrap();
        let grandchild_snapshot = fixture
            .registry
            .get_runtime_snapshot(grandchild.agent.runtime_snapshot_id)
            .await
            .unwrap();
        assert_eq!(
            child_snapshot.parent_snapshot_id,
            Some(fixture.root.runtime_snapshot_id)
        );
        assert_eq!(
            grandchild_snapshot.parent_snapshot_id,
            Some(child_snapshot.id)
        );

        let message = child_invocation
            .send_message("/root", "evidence ready".to_string(), Some(Uuid::new_v4()))
            .await
            .unwrap();
        assert_eq!(message.to_agent_thread_id, fixture.root.id);

        let agents = root.list_agents(Some("/root/research")).await.unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents.iter().all(|item| item.activity.is_some()));

        let denied = child_invocation.interrupt_agent("/root").await.unwrap_err();
        assert!(matches!(
            denied,
            AgentCollaborationInvocationError::Domain(
                CollaborationDomainError::LifecyclePermissionDenied { .. }
            )
        ));
        let interrupted = root
            .interrupt_agent("/root/research/review")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(interrupted.id, grandchild.turn.id);

        fixture
            .registry
            .transition_turn(grandchild.turn.id, AgentTurnStatus::Running)
            .await
            .unwrap();
        fixture
            .registry
            .transition_turn(grandchild.turn.id, AgentTurnStatus::Completed)
            .await
            .unwrap();
        let followup = root
            .followup_task(
                "/root/research/review",
                "verify the remaining edge case".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(followup.sequence, 2);

        let waited = root
            .wait_agent(WaitAgentRequest {
                target: Some("/root/research/review".to_string()),
                after_cursor: Some(8),
                timeout: Duration::ZERO,
                reasoning_tail_chars: 100,
                tool_result_chars: 100,
                event_limit: 4,
            })
            .await
            .unwrap();
        assert!(!waited.timed_out);
        assert_eq!(waited.agent.id, grandchild.agent.id);
        assert_eq!(waited.activity.unwrap().cursor, 9);
        assert_eq!(waited.messages.len(), 1);

        let commands = fixture.scheduler.commands.lock().unwrap();
        assert!(commands.iter().any(|command| matches!(
            command,
            AgentRunCommand::Cancel { agent_thread_id, .. }
                if *agent_thread_id == grandchild.agent.id
        )));
    }

    #[tokio::test]
    async fn persisted_identity_cannot_be_spoofed_by_tool_arguments_or_cross_session_uuid() {
        let fixture = Fixture::new().await;
        let root = fixture.invocation(&fixture.root, &fixture.root_turn);

        let other_registry = InMemoryCollaborationRegistry::new();
        let (_, other_root, _) = other_registry
            .create_session(CreateCollaborationSession {
                user_task_id: Uuid::new_v4(),
                root_turn_id: AgentTurnId::new(),
                root_task_message: "other".to_string(),
                root_agent_type: "default".to_string(),
                root_runtime_snapshot: RuntimeSnapshotSeed::new(
                    None,
                    test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                session_policy: CollaborationSessionPolicy::default(),
                root_spawn_policy: AgentSpawnPolicy::allows_children(1, 1),
            })
            .await
            .unwrap();
        let error = root
            .send_message(&other_root.id.to_string(), "probe".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            AgentCollaborationInvocationError::TargetNotFound(_)
        ));

        let invalid = AgentCollaborationInvocation::new(
            fixture.runtime,
            fixture.activity,
            fixture.deriver,
            AgentInvocationIdentity {
                runtime_snapshot_id: RuntimeSnapshotId::new(),
                ..root.identity()
            },
        );
        assert!(matches!(
            invalid.list_agents(None).await.unwrap_err(),
            AgentCollaborationInvocationError::InvalidInvocationIdentity
        ));
    }
}
