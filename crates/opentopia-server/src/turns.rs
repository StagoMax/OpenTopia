use chrono::Utc;
use opentopia_core::collaboration::{
    AgentMailboxNotifier, AgentTurnId, AgentTurnStatus, SqliteAgentActivitySource,
    SqliteCollaborationRepository,
};
use opentopia_core::{SessionStore, SqliteSessionStore, TurnRecord, TurnStatus};
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct TurnManager {
    store: Arc<SqliteSessionStore>,
    running: Arc<RwLock<HashMap<Uuid, RunningTurn>>>,
}

/// Root-Agent lifecycle facade.
///
/// `AgentTurnRecord` is the authoritative execution state for root and
/// descendant Agents alike. `TurnManager` owns only the user-facing product
/// projection plus the process-local cancellation handle required by the HTTP
/// adapter. Every root transition is committed to the canonical AgentTurn
/// before its product projection is updated.
#[derive(Clone)]
pub struct RootTurnLifecycle {
    projection: TurnManager,
    repository: Arc<SqliteCollaborationRepository>,
    activity: Arc<SqliteAgentActivitySource>,
    mailbox_notifier: Arc<dyn AgentMailboxNotifier>,
}

#[derive(Clone)]
struct RunningTurn {
    record: TurnRecord,
    cancel: CancellationToken,
}

#[derive(Clone)]
pub struct TurnHandle {
    pub turn_id: Uuid,
    pub invocation_id: u64,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCancelResult {
    pub turn_id: Option<Uuid>,
    pub cancelled: bool,
    pub message: String,
}

impl TurnManager {
    fn new(store: Arc<SqliteSessionStore>) -> Self {
        Self {
            store,
            running: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn begin(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
    ) -> anyhow::Result<Result<TurnHandle, TurnRecord>> {
        let mut running = self.running.write().expect("turn manager poisoned");
        if let Some(active) = running.get(&thread_id) {
            return Ok(Err(in_memory_status(active)));
        }

        let record = TurnRecord::running(thread_id, user_message_id);
        let record = match self.store.insert_turn(record) {
            Ok(record) => record,
            Err(error) => {
                if let Some(active) = self.store.get_active_turn(thread_id)? {
                    return Ok(Err(active));
                }
                return Err(error);
            }
        };
        let cancel = CancellationToken::new();
        running.insert(
            thread_id,
            RunningTurn {
                record: record.clone(),
                cancel: cancel.clone(),
            },
        );
        Ok(Ok(TurnHandle {
            turn_id: record.turn_id,
            invocation_id: record.invocation_id,
            cancel,
        }))
    }

    /// Product-only resume retained for projection unit tests. Runtime code
    /// resumes root Turns through `RootTurnLifecycle::resume`, which commits
    /// the canonical AgentTurn first.
    #[cfg(test)]
    fn resume(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        user_message_id: Uuid,
    ) -> anyhow::Result<Result<TurnHandle, TurnRecord>> {
        let store = self.store.clone();
        self.resume_from_authority(
            thread_id,
            turn_id,
            user_message_id,
            move || {
                let persisted = store
                    .get_turn(turn_id)?
                    .ok_or_else(|| anyhow::anyhow!("turn not found: {turn_id}"))?;
                Ok(persisted.invocation_id.saturating_add(1))
            },
            || {},
        )
    }

    fn resume_from_authority<F, R>(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        user_message_id: Uuid,
        resume_authority: F,
        rollback_authority: R,
    ) -> anyhow::Result<Result<TurnHandle, TurnRecord>>
    where
        F: FnOnce() -> anyhow::Result<u64>,
        R: FnOnce(),
    {
        let mut running = self.running.write().expect("turn manager poisoned");
        if let Some(active) = running.get(&thread_id) {
            return Ok(Err(in_memory_status(active)));
        }
        let persisted = self
            .store
            .get_turn(turn_id)?
            .ok_or_else(|| anyhow::anyhow!("turn not found: {turn_id}"))?;
        anyhow::ensure!(
            persisted.thread_id == thread_id,
            "turn belongs to another thread"
        );
        anyhow::ensure!(
            persisted.user_message_id == user_message_id,
            "continuation belongs to another user message"
        );

        // Admission is checked while the in-memory turn lock is held, then the
        // canonical transition is committed before the product projection.
        let invocation_id = resume_authority()?;
        let mut record = match self.store.resume_turn_invocation(turn_id) {
            Ok(Some(record)) => record,
            Ok(None) => {
                rollback_authority();
                anyhow::bail!("turn {turn_id} projection is not resumable");
            }
            Err(error) => {
                rollback_authority();
                return Err(error);
            }
        };
        record.invocation_id = invocation_id;
        let cancel = CancellationToken::new();
        running.insert(
            thread_id,
            RunningTurn {
                record,
                cancel: cancel.clone(),
            },
        );
        Ok(Ok(TurnHandle {
            turn_id,
            invocation_id,
            cancel,
        }))
    }

    fn status(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let mut latest = self.store.get_latest_turn(thread_id)?;
        let running = self.running.read().expect("turn manager poisoned");
        if let (Some(record), Some(active)) = (&mut latest, running.get(&thread_id)) {
            if record.turn_id == active.record.turn_id && active.cancel.is_cancelled() {
                record.status = TurnStatus::Cancelling;
            }
        }
        Ok(latest)
    }

    fn cancel(
        &self,
        thread_id: Uuid,
        requested_turn_id: Option<Uuid>,
    ) -> anyhow::Result<TurnCancelResult> {
        let mut running = self.running.write().expect("turn manager poisoned");
        let Some(active) = running.get_mut(&thread_id) else {
            return Ok(TurnCancelResult {
                turn_id: requested_turn_id,
                cancelled: false,
                message: "no active agent turn".to_string(),
            });
        };
        if requested_turn_id.is_some_and(|turn_id| turn_id != active.record.turn_id) {
            return Ok(TurnCancelResult {
                turn_id: requested_turn_id,
                cancelled: false,
                message: format!("active agent turn is {}", active.record.turn_id),
            });
        }

        let Some(record) =
            self.store
                .update_turn_status(active.record.turn_id, TurnStatus::Cancelling, None)?
        else {
            return Ok(TurnCancelResult {
                turn_id: Some(active.record.turn_id),
                cancelled: false,
                message: "active agent turn is no longer persisted".to_string(),
            });
        };
        active.record = record;
        active.cancel.cancel();
        Ok(TurnCancelResult {
            turn_id: Some(active.record.turn_id),
            cancelled: true,
            message: "agent turn cancellation requested".to_string(),
        })
    }

    fn finish(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>> {
        anyhow::ensure!(
            !status.is_active(),
            "finish requires a paused or terminal turn status"
        );
        let mut running = self.running.write().expect("turn manager poisoned");
        let Some(active) = running
            .get(&thread_id)
            .filter(|active| active.record.turn_id == turn_id)
        else {
            return Ok(None);
        };

        let status = if active.cancel.is_cancelled() {
            TurnStatus::Cancelled
        } else {
            status
        };
        let error = if status == TurnStatus::Cancelled {
            None
        } else {
            error
        };

        let update = self.store.update_turn_status(turn_id, status, error);
        running.remove(&thread_id);
        update
    }

    fn effective_finish_status(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        requested: TurnStatus,
    ) -> TurnStatus {
        self.running
            .read()
            .expect("turn manager poisoned")
            .get(&thread_id)
            .filter(|active| active.record.turn_id == turn_id)
            .is_some_and(|active| active.cancel.is_cancelled())
            .then_some(TurnStatus::Cancelled)
            .unwrap_or(requested)
    }

    fn project_status(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>> {
        let record = self.store.update_turn_status(turn_id, status, error)?;
        if !status.is_active() {
            let mut running = self.running.write().expect("turn manager poisoned");
            if running
                .get(&thread_id)
                .is_some_and(|active| active.record.turn_id == turn_id)
            {
                running.remove(&thread_id);
            }
        }
        Ok(record)
    }
}

impl RootTurnLifecycle {
    pub fn new(
        store: Arc<SqliteSessionStore>,
        repository: Arc<SqliteCollaborationRepository>,
        activity: Arc<SqliteAgentActivitySource>,
        mailbox_notifier: Arc<dyn AgentMailboxNotifier>,
    ) -> Self {
        Self {
            projection: TurnManager::new(store),
            repository,
            activity,
            mailbox_notifier,
        }
    }

    pub fn begin(
        &self,
        thread_id: Uuid,
        user_message_id: Uuid,
    ) -> anyhow::Result<Result<TurnHandle, TurnRecord>> {
        // This reserves the public Turn identity. The canonical AgentTurn is
        // created from that exact ID by bind_root_collaboration before model
        // execution begins.
        self.projection.begin(thread_id, user_message_id)
    }

    pub fn resume(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        user_message_id: Uuid,
    ) -> anyhow::Result<Result<TurnHandle, TurnRecord>> {
        let canonical_id = AgentTurnId::from_uuid(turn_id);
        let canonical = self
            .repository
            .find_turn(canonical_id)?
            .ok_or_else(|| anyhow::anyhow!("canonical AgentTurn was not found: {canonical_id}"))?;
        anyhow::ensure!(
            canonical.status.needs_attention(),
            "canonical AgentTurn is not waiting for a resumable interaction"
        );
        let waiting_status = canonical.status;
        self.projection.resume_from_authority(
            thread_id,
            turn_id,
            user_message_id,
            || {
                let resumed = self.repository.resume_turn(canonical.id)?;
                self.activity.notify(resumed.agent_thread_id);
                Ok(resumed.invocation_id)
            },
            || {
                let rollback = self.repository.record_turn_state(
                    canonical.id,
                    waiting_status,
                    &json!({
                        "status": waiting_status,
                        "agentTurnId": canonical.id,
                        "reason": "root product projection failed to resume",
                    }),
                );
                if let Err(error) = rollback {
                    tracing::error!(
                        ?error,
                        agent_turn_id = %canonical.id,
                        "failed to roll canonical AgentTurn back after projection resume failure"
                    );
                }
                self.activity.notify(canonical.agent_thread_id);
            },
        )
    }

    pub fn status(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let Some(mut projection) = self.projection.status(thread_id)? else {
            return Ok(None);
        };
        let Some(canonical) = self
            .repository
            .find_turn(AgentTurnId::from_uuid(projection.turn_id))?
        else {
            // A Turn can fail during request preflight before it becomes an
            // executable AgentTurn. Such records remain product-only facts.
            return Ok(Some(projection));
        };

        projection.invocation_id = canonical.invocation_id;
        let canonical_projection = product_status(canonical.status);
        let cancellation_requested = projection.status == TurnStatus::Cancelling
            && canonical.status == AgentTurnStatus::Running;
        if !cancellation_requested && projection.status != canonical_projection {
            let error = matches!(
                canonical_projection,
                TurnStatus::Failed | TurnStatus::Interrupted
            )
            .then(|| projection.error.clone())
            .flatten();
            if let Some(mut repaired) = self.projection.project_status(
                thread_id,
                projection.turn_id,
                canonical_projection,
                error,
            )? {
                repaired.invocation_id = canonical.invocation_id;
                projection = repaired;
            } else {
                projection.status = canonical_projection;
            }
        }
        Ok(Some(projection))
    }

    pub fn cancel(
        &self,
        thread_id: Uuid,
        requested_turn_id: Option<Uuid>,
    ) -> anyhow::Result<TurnCancelResult> {
        // Cancellation is a control signal. The canonical state remains
        // Running until the Agent kernel reaches cancellation and commits the
        // terminal Cancelled transition.
        self.projection.cancel(thread_id, requested_turn_id)
    }

    pub fn finish(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        requested_status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>> {
        anyhow::ensure!(
            !requested_status.is_active(),
            "finish requires a paused or terminal turn status"
        );
        let effective_status =
            self.projection
                .effective_finish_status(thread_id, turn_id, requested_status);
        let effective_error = (effective_status != TurnStatus::Cancelled)
            .then(|| error.clone())
            .flatten();

        if let Some(canonical) = self.repository.find_turn(AgentTurnId::from_uuid(turn_id))? {
            let canonical_status = agent_status(effective_status).ok_or_else(|| {
                anyhow::anyhow!(
                    "product status {} has no canonical AgentTurn state",
                    effective_status.as_str()
                )
            })?;
            let payload = json!({
                "status": canonical_status,
                "agentTurnId": canonical.id,
                "error": effective_error,
            });
            if let Some(message) =
                self.repository
                    .record_turn_state(canonical.id, canonical_status, &payload)?
            {
                self.mailbox_notifier.message_enqueued(&message);
            }
            self.activity.notify(canonical.agent_thread_id);
        }

        // Projection failure cannot roll back the canonical transition. A
        // later status read repairs the projection from AgentTurn.
        match self.projection.finish(
            thread_id,
            turn_id,
            effective_status,
            effective_error.clone(),
        )? {
            Some(record) => Ok(Some(record)),
            None => self.projection.project_status(
                thread_id,
                turn_id,
                effective_status,
                effective_error,
            ),
        }
    }
}

fn product_status(status: AgentTurnStatus) -> TurnStatus {
    match status {
        AgentTurnStatus::Queued | AgentTurnStatus::Running => TurnStatus::Running,
        AgentTurnStatus::WaitingApproval => TurnStatus::WaitingApproval,
        AgentTurnStatus::WaitingInput => TurnStatus::WaitingUserInput,
        AgentTurnStatus::WaitingAction => TurnStatus::WaitingUserAction,
        AgentTurnStatus::Completed => TurnStatus::Succeeded,
        AgentTurnStatus::Failed => TurnStatus::Failed,
        AgentTurnStatus::Cancelled => TurnStatus::Cancelled,
        AgentTurnStatus::Interrupted => TurnStatus::Interrupted,
    }
}

fn agent_status(status: TurnStatus) -> Option<AgentTurnStatus> {
    match status {
        TurnStatus::Running | TurnStatus::Cancelling => None,
        TurnStatus::WaitingApproval => Some(AgentTurnStatus::WaitingApproval),
        TurnStatus::WaitingUserInput => Some(AgentTurnStatus::WaitingInput),
        TurnStatus::WaitingUserAction => Some(AgentTurnStatus::WaitingAction),
        TurnStatus::Succeeded => Some(AgentTurnStatus::Completed),
        TurnStatus::Failed => Some(AgentTurnStatus::Failed),
        TurnStatus::Cancelled => Some(AgentTurnStatus::Cancelled),
        TurnStatus::Interrupted => Some(AgentTurnStatus::Interrupted),
    }
}

fn in_memory_status(turn: &RunningTurn) -> TurnRecord {
    let mut record = turn.record.clone();
    if turn.cancel.is_cancelled() {
        record.status = TurnStatus::Cancelling;
        record.updated_at = Utc::now();
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentopia_core::collaboration::{
        AgentSpawnPolicy, CollaborationRegistry, CollaborationSessionPolicy,
        CreateCollaborationSession, NoopAgentMailboxNotifier, RuntimeSnapshotSeed,
    };
    use serde_json::json;
    use std::path::PathBuf;

    fn manager_with_thread() -> (TurnManager, Arc<SqliteSessionStore>, Uuid) {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/turn-manager"))
            .expect("create thread");
        (TurnManager::new(store.clone()), store, thread.id)
    }

    async fn lifecycle_with_root() -> (
        RootTurnLifecycle,
        Arc<SqliteSessionStore>,
        Arc<SqliteCollaborationRepository>,
        Uuid,
        TurnHandle,
    ) {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/root-lifecycle"))
            .expect("create thread");
        let repository = Arc::new(
            SqliteCollaborationRepository::new(store.clone()).expect("collaboration repository"),
        );
        let activity = Arc::new(SqliteAgentActivitySource::new(repository.clone()));
        let lifecycle = RootTurnLifecycle::new(
            store.clone(),
            repository.clone(),
            activity,
            Arc::new(NoopAgentMailboxNotifier),
        );
        let handle = lifecycle
            .begin(thread.id, Uuid::new_v4())
            .expect("begin product projection")
            .expect("root turn starts");
        let (_, _, root_turn) = repository
            .create_session(CreateCollaborationSession {
                user_task_id: thread.id,
                root_turn_id: AgentTurnId::from_uuid(handle.turn_id),
                root_task_message: "root lifecycle test".to_string(),
                root_agent_type: "default".to_string(),
                root_runtime_snapshot: RuntimeSnapshotSeed::new(None, json!({})),
                session_policy: CollaborationSessionPolicy::default(),
                root_spawn_policy: AgentSpawnPolicy::default(),
            })
            .await
            .expect("create canonical root");
        repository
            .transition_turn(root_turn.id, AgentTurnStatus::Running)
            .await
            .expect("run canonical root");
        (lifecycle, store, repository, thread.id, handle)
    }

    #[test]
    fn serializes_turns_per_thread_and_cancels_matching_turn() {
        let (manager, store, thread_id) = manager_with_thread();
        let first = manager
            .begin(thread_id, Uuid::new_v4())
            .expect("begin turn")
            .expect("first turn starts");
        assert!(manager
            .begin(thread_id, Uuid::new_v4())
            .expect("begin conflicting turn")
            .is_err());

        let wrong = manager
            .cancel(thread_id, Some(Uuid::new_v4()))
            .expect("cancel wrong turn");
        assert!(!wrong.cancelled);
        assert!(!first.cancel.is_cancelled());

        let cancelled = manager
            .cancel(thread_id, Some(first.turn_id))
            .expect("cancel matching turn");
        assert!(cancelled.cancelled);
        assert!(first.cancel.is_cancelled());
        assert_eq!(
            manager
                .status(thread_id)
                .expect("get status")
                .expect("turn status")
                .status,
            TurnStatus::Cancelling
        );

        manager
            .finish(thread_id, first.turn_id, TurnStatus::Succeeded, None)
            .expect("finish cancelled turn");
        assert_eq!(
            manager
                .status(thread_id)
                .expect("get latest status")
                .expect("latest turn")
                .status,
            TurnStatus::Cancelled
        );
        assert_eq!(
            store
                .get_turn(first.turn_id)
                .expect("read persisted turn")
                .expect("persisted turn")
                .status,
            TurnStatus::Cancelled
        );
    }

    #[test]
    fn finish_persists_waiting_and_success_states() {
        let (manager, _store, thread_id) = manager_with_thread();
        let user_message_id = Uuid::new_v4();
        let paused = manager
            .begin(thread_id, user_message_id)
            .expect("begin paused turn")
            .expect("paused turn starts");
        manager
            .finish(thread_id, paused.turn_id, TurnStatus::WaitingApproval, None)
            .expect("pause turn");
        assert_eq!(
            manager
                .status(thread_id)
                .expect("get paused status")
                .expect("paused status")
                .status,
            TurnStatus::WaitingApproval
        );

        let resumed = manager
            .resume(thread_id, paused.turn_id, user_message_id)
            .expect("resume turn")
            .expect("resumed turn starts");
        assert_eq!(resumed.turn_id, paused.turn_id);
        assert_eq!(paused.invocation_id, 1);
        assert_eq!(resumed.invocation_id, 2);
        manager
            .finish(thread_id, resumed.turn_id, TurnStatus::Succeeded, None)
            .expect("finish resumed turn");
        assert_eq!(
            manager
                .status(thread_id)
                .expect("get succeeded status")
                .expect("succeeded status")
                .status,
            TurnStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn root_lifecycle_uses_agent_turn_as_authority_and_repairs_projection_drift() {
        let (lifecycle, store, repository, thread_id, first) = lifecycle_with_root().await;
        lifecycle
            .finish(thread_id, first.turn_id, TurnStatus::WaitingApproval, None)
            .expect("pause root")
            .expect("waiting product projection");
        assert_eq!(
            repository
                .get_turn(AgentTurnId::from_uuid(first.turn_id))
                .await
                .expect("canonical waiting turn")
                .status,
            AgentTurnStatus::WaitingApproval
        );

        store
            .update_turn_status(first.turn_id, TurnStatus::Failed, Some("stale".to_string()))
            .expect("corrupt product projection");
        let repaired = lifecycle
            .status(thread_id)
            .expect("read authority-backed status")
            .expect("root status");
        assert_eq!(repaired.status, TurnStatus::WaitingApproval);
        assert_eq!(
            store
                .get_turn(first.turn_id)
                .expect("read repaired projection")
                .expect("projection exists")
                .status,
            TurnStatus::WaitingApproval
        );

        let error = match lifecycle.resume(thread_id, first.turn_id, first.turn_id) {
            Err(error) => error,
            Ok(_) => panic!("wrong user-message identity must be rejected before canonical resume"),
        };
        assert!(error.to_string().contains("continuation belongs"));
        let user_message_id = store
            .get_turn(first.turn_id)
            .unwrap()
            .unwrap()
            .user_message_id;
        let resumed = lifecycle
            .resume(thread_id, first.turn_id, user_message_id)
            .expect("resume root")
            .expect("root is admitted");
        assert_eq!(resumed.invocation_id, 2);
        assert_eq!(
            repository
                .get_turn(AgentTurnId::from_uuid(first.turn_id))
                .await
                .unwrap()
                .invocation_id,
            2
        );
        lifecycle
            .finish(thread_id, first.turn_id, TurnStatus::Succeeded, None)
            .expect("complete root");
        let canonical = repository
            .get_turn(AgentTurnId::from_uuid(first.turn_id))
            .await
            .unwrap();
        assert_eq!(canonical.status, AgentTurnStatus::Completed);
        let outcomes = repository
            .list_ledger_items(canonical.agent_thread_id, "turn_outcome")
            .expect("canonical outcome ledger");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0]["status"], "completed");
        assert_eq!(
            lifecycle.status(thread_id).unwrap().unwrap().status,
            TurnStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn root_cancellation_commits_canonical_cancelled_before_product_projection() {
        let (lifecycle, store, repository, thread_id, turn) = lifecycle_with_root().await;
        let cancelled = lifecycle
            .cancel(thread_id, Some(turn.turn_id))
            .expect("request cancellation");
        assert!(cancelled.cancelled);
        lifecycle
            .finish(thread_id, turn.turn_id, TurnStatus::Succeeded, None)
            .expect("finish cancelled root");
        assert_eq!(
            repository
                .get_turn(AgentTurnId::from_uuid(turn.turn_id))
                .await
                .unwrap()
                .status,
            AgentTurnStatus::Cancelled
        );
        assert_eq!(
            store.get_turn(turn.turn_id).unwrap().unwrap().status,
            TurnStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn root_resume_rolls_canonical_state_back_when_projection_resume_fails() {
        let (lifecycle, store, repository, thread_id, turn) = lifecycle_with_root().await;
        lifecycle
            .finish(thread_id, turn.turn_id, TurnStatus::WaitingUserInput, None)
            .expect("pause root");
        let user_message_id = store
            .get_turn(turn.turn_id)
            .unwrap()
            .unwrap()
            .user_message_id;
        store
            .update_turn_status(turn.turn_id, TurnStatus::Succeeded, None)
            .expect("make projection non-resumable");

        assert!(lifecycle
            .resume(thread_id, turn.turn_id, user_message_id)
            .is_err());
        assert_eq!(
            repository
                .get_turn(AgentTurnId::from_uuid(turn.turn_id))
                .await
                .unwrap()
                .status,
            AgentTurnStatus::WaitingInput,
            "canonical state must roll back when its product projection cannot resume"
        );
    }

    #[tokio::test]
    async fn rejected_canonical_transition_never_advances_the_product_projection() {
        let (lifecycle, store, repository, thread_id, turn) = lifecycle_with_root().await;
        repository
            .transition_turn(
                AgentTurnId::from_uuid(turn.turn_id),
                AgentTurnStatus::Completed,
            )
            .await
            .expect("complete canonical turn externally");
        let error = lifecycle
            .finish(thread_id, turn.turn_id, TurnStatus::WaitingApproval, None)
            .expect_err("terminal canonical turn rejects a waiting transition");
        assert!(
            error.to_string().contains("transition"),
            "unexpected canonical rejection: {error}"
        );
        assert_eq!(
            store.get_turn(turn.turn_id).unwrap().unwrap().status,
            TurnStatus::Running,
            "projection must not move when the canonical transition fails"
        );
        assert_eq!(
            lifecycle.status(thread_id).unwrap().unwrap().status,
            TurnStatus::Succeeded,
            "status reads repair the projection from canonical state"
        );
    }
}
