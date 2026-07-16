use chrono::Utc;
use opentopia_core::{SessionStore, SqliteSessionStore, TurnRecord, TurnStatus};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct TurnManager {
    store: Arc<SqliteSessionStore>,
    running: Arc<RwLock<HashMap<Uuid, RunningTurn>>>,
}

#[derive(Clone)]
struct RunningTurn {
    record: TurnRecord,
    cancel: CancellationToken,
}

#[derive(Clone)]
pub struct TurnHandle {
    pub turn_id: Uuid,
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
    pub fn new(store: Arc<SqliteSessionStore>) -> Self {
        Self {
            store,
            running: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn begin(
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
            cancel,
        }))
    }

    pub fn status(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let mut latest = self.store.get_latest_turn(thread_id)?;
        let running = self.running.read().expect("turn manager poisoned");
        if let (Some(record), Some(active)) = (&mut latest, running.get(&thread_id)) {
            if record.turn_id == active.record.turn_id && active.cancel.is_cancelled() {
                record.status = TurnStatus::Cancelling;
            }
        }
        Ok(latest)
    }

    pub fn cancel(
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

    pub fn finish(
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
    use std::path::PathBuf;

    fn manager_with_thread() -> (TurnManager, Arc<SqliteSessionStore>, Uuid) {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/turn-manager"))
            .expect("create thread");
        (TurnManager::new(store.clone()), store, thread.id)
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
        let paused = manager
            .begin(thread_id, Uuid::new_v4())
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
            .begin(thread_id, Uuid::new_v4())
            .expect("begin resumed turn")
            .expect("resumed turn starts");
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
}
