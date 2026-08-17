//! Runtime-only messages delivered to a turn at explicit safe points.
//!
//! Queue keys are control-plane identities and must never be materialized into
//! the cacheable prompt prefix. Persisting an async result after its owning turn
//! has ended remains the conversation-ledger adapter's responsibility.

use crate::collaboration::AgentMailboxMessage;
use crate::tool_runtime::AsyncToolResult;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnInboxItem {
    Steer {
        message_id: Uuid,
        content: String,
    },
    AsyncToolResult {
        result: AsyncToolResult,
    },
    Reminder {
        source_id: String,
        message: String,
    },
    /// Durable peer data materialized as an untrusted synthetic tool
    /// observation at the next model safe point.
    AgentMessage {
        message: AgentMailboxMessage,
    },
    Cancel,
}

/// Multi-producer queue read only by the kernel at a safe point.
pub trait TurnInbox: Send + Sync {
    fn push(&self, turn_id: Uuid, item: TurnInboxItem);
    fn drain(&self, turn_id: Uuid) -> Vec<TurnInboxItem>;
}

#[derive(Debug, Default)]
pub struct BufferedTurnInbox {
    queues: Mutex<HashMap<Uuid, VecDeque<TurnInboxItem>>>,
}

impl TurnInbox for BufferedTurnInbox {
    fn push(&self, turn_id: Uuid, item: TurnInboxItem) {
        self.queues
            .lock()
            .expect("turn inbox mutex poisoned")
            .entry(turn_id)
            .or_default()
            .push_back(item);
    }

    fn drain(&self, turn_id: Uuid) -> Vec<TurnInboxItem> {
        self.queues
            .lock()
            .expect("turn inbox mutex poisoned")
            .remove(&turn_id)
            .unwrap_or_default()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_point_drain_is_ordered_and_turn_scoped() {
        let inbox: &dyn TurnInbox = &BufferedTurnInbox::default();
        let first_turn = Uuid::new_v4();
        let second_turn = Uuid::new_v4();
        inbox.push(
            first_turn,
            TurnInboxItem::Reminder {
                source_id: "one".to_string(),
                message: "first".to_string(),
            },
        );
        inbox.push(first_turn, TurnInboxItem::Cancel);
        inbox.push(
            second_turn,
            TurnInboxItem::Reminder {
                source_id: "two".to_string(),
                message: "second".to_string(),
            },
        );

        assert!(matches!(
            inbox.drain(first_turn).as_slice(),
            [TurnInboxItem::Reminder { .. }, TurnInboxItem::Cancel]
        ));
        assert!(inbox.drain(first_turn).is_empty());
        assert_eq!(inbox.drain(second_turn).len(), 1);
    }
}
