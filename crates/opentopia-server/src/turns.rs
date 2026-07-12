use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct TurnManager {
    running: Arc<RwLock<HashMap<Uuid, RunningTurn>>>,
}

#[derive(Clone)]
struct RunningTurn {
    turn_id: Uuid,
    user_message_id: Uuid,
    started_at: DateTime<Utc>,
    cancel: CancellationToken,
}

#[derive(Clone)]
pub struct TurnHandle {
    pub turn_id: Uuid,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStatus {
    pub turn_id: Uuid,
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub status: &'static str,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCancelResult {
    pub turn_id: Option<Uuid>,
    pub cancelled: bool,
    pub message: String,
}

impl TurnManager {
    pub fn begin(&self, thread_id: Uuid, user_message_id: Uuid) -> Result<TurnHandle, TurnStatus> {
        let mut running = self.running.write().expect("turn manager poisoned");
        if let Some(active) = running.get(&thread_id) {
            return Err(to_status(thread_id, active));
        }

        let turn_id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        running.insert(
            thread_id,
            RunningTurn {
                turn_id,
                user_message_id,
                started_at: Utc::now(),
                cancel: cancel.clone(),
            },
        );
        Ok(TurnHandle { turn_id, cancel })
    }

    pub fn status(&self, thread_id: Uuid) -> Option<TurnStatus> {
        self.running
            .read()
            .expect("turn manager poisoned")
            .get(&thread_id)
            .map(|turn| to_status(thread_id, turn))
    }

    pub fn cancel(&self, thread_id: Uuid, requested_turn_id: Option<Uuid>) -> TurnCancelResult {
        let running = self.running.read().expect("turn manager poisoned");
        let Some(active) = running.get(&thread_id) else {
            return TurnCancelResult {
                turn_id: requested_turn_id,
                cancelled: false,
                message: "no active agent turn".to_string(),
            };
        };
        if requested_turn_id.is_some_and(|turn_id| turn_id != active.turn_id) {
            return TurnCancelResult {
                turn_id: requested_turn_id,
                cancelled: false,
                message: format!("active agent turn is {}", active.turn_id),
            };
        }
        active.cancel.cancel();
        TurnCancelResult {
            turn_id: Some(active.turn_id),
            cancelled: true,
            message: "agent turn cancellation requested".to_string(),
        }
    }

    pub fn finish(&self, thread_id: Uuid, turn_id: Uuid) {
        let mut running = self.running.write().expect("turn manager poisoned");
        if running
            .get(&thread_id)
            .is_some_and(|active| active.turn_id == turn_id)
        {
            running.remove(&thread_id);
        }
    }
}

fn to_status(thread_id: Uuid, turn: &RunningTurn) -> TurnStatus {
    TurnStatus {
        turn_id: turn.turn_id,
        thread_id,
        user_message_id: turn.user_message_id,
        status: if turn.cancel.is_cancelled() {
            "cancelling"
        } else {
            "running"
        },
        started_at: turn.started_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_turns_per_thread_and_cancels_matching_turn() {
        let manager = TurnManager::default();
        let thread_id = Uuid::new_v4();
        let first = manager
            .begin(thread_id, Uuid::new_v4())
            .expect("first turn starts");
        assert!(manager.begin(thread_id, Uuid::new_v4()).is_err());

        let wrong = manager.cancel(thread_id, Some(Uuid::new_v4()));
        assert!(!wrong.cancelled);
        assert!(!first.cancel.is_cancelled());

        let cancelled = manager.cancel(thread_id, Some(first.turn_id));
        assert!(cancelled.cancelled);
        assert!(first.cancel.is_cancelled());
        manager.finish(thread_id, first.turn_id);
        assert!(manager.status(thread_id).is_none());
    }
}
