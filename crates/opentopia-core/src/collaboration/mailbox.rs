use super::{AgentMailboxMessageId, AgentThreadId, CollaborationSessionId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentMailboxMessageKind {
    Message,
    Completion,
    NeedsAttention,
}

impl AgentMailboxMessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Completion => "completion",
            Self::NeedsAttention => "needs_attention",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AgentMailboxError> {
        match value {
            "message" => Ok(Self::Message),
            "completion" => Ok(Self::Completion),
            "needs_attention" => Ok(Self::NeedsAttention),
            other => Err(AgentMailboxError::Persistence(format!(
                "unknown mailbox message kind `{other}`"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMailboxMessage {
    pub id: AgentMailboxMessageId,
    pub session_id: CollaborationSessionId,
    pub sequence: u64,
    pub from_agent_thread_id: AgentThreadId,
    pub to_agent_thread_id: AgentThreadId,
    pub kind: AgentMailboxMessageKind,
    pub payload: Value,
    pub causation_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub acknowledged_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct EnqueueAgentMessage {
    pub session_id: CollaborationSessionId,
    pub from_agent_thread_id: AgentThreadId,
    pub to_agent_thread_id: AgentThreadId,
    pub kind: AgentMailboxMessageKind,
    pub payload: Value,
    pub causation_id: Option<Uuid>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentMailboxError {
    #[error("mailbox message not found: {0}")]
    MessageNotFound(AgentMailboxMessageId),
    #[error("mailbox message {message_id} does not belong to target agent {target}")]
    WrongTarget {
        message_id: AgentMailboxMessageId,
        target: AgentThreadId,
    },
    #[error("mailbox lock is poisoned")]
    Poisoned,
    #[error("mailbox persistence failed: {0}")]
    Persistence(String),
}

#[async_trait]
pub trait AgentMailbox: Send + Sync {
    async fn enqueue(
        &self,
        request: EnqueueAgentMessage,
    ) -> Result<AgentMailboxMessage, AgentMailboxError>;

    async fn snapshot(
        &self,
        session_id: CollaborationSessionId,
        target: AgentThreadId,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<AgentMailboxMessage>, AgentMailboxError>;

    async fn acknowledge(
        &self,
        target: AgentThreadId,
        message_ids: &[AgentMailboxMessageId],
    ) -> Result<(), AgentMailboxError>;
}

/// Best-effort live delivery after the durable enqueue commits. Missing a
/// wake-up never loses data because the next invocation re-snapshots mailbox.
pub trait AgentMailboxNotifier: Send + Sync {
    fn message_enqueued(&self, message: &AgentMailboxMessage);
}

#[derive(Debug, Default)]
pub struct NoopAgentMailboxNotifier;

impl AgentMailboxNotifier for NoopAgentMailboxNotifier {
    fn message_enqueued(&self, _message: &AgentMailboxMessage) {}
}

#[derive(Default)]
struct MailboxState {
    next_sequence: HashMap<CollaborationSessionId, u64>,
    messages: Vec<AgentMailboxMessage>,
    deduplication: HashMap<
        (
            CollaborationSessionId,
            AgentThreadId,
            AgentMailboxMessageKind,
            Uuid,
        ),
        AgentMailboxMessageId,
    >,
}

#[derive(Clone, Default)]
pub struct InMemoryAgentMailbox {
    state: Arc<Mutex<MailboxState>>,
}

impl InMemoryAgentMailbox {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AgentMailbox for InMemoryAgentMailbox {
    async fn enqueue(
        &self,
        request: EnqueueAgentMessage,
    ) -> Result<AgentMailboxMessage, AgentMailboxError> {
        let mut state = self.state.lock().map_err(|_| AgentMailboxError::Poisoned)?;
        if let Some(causation_id) = request.causation_id {
            let key = (
                request.session_id,
                request.to_agent_thread_id,
                request.kind,
                causation_id,
            );
            if let Some(existing_id) = state.deduplication.get(&key) {
                return state
                    .messages
                    .iter()
                    .find(|message| message.id == *existing_id)
                    .cloned()
                    .ok_or(AgentMailboxError::MessageNotFound(*existing_id));
            }
        }

        let sequence = state
            .next_sequence
            .entry(request.session_id)
            .and_modify(|sequence| *sequence += 1)
            .or_insert(1)
            .to_owned();
        let message = AgentMailboxMessage {
            id: AgentMailboxMessageId::new(),
            session_id: request.session_id,
            sequence,
            from_agent_thread_id: request.from_agent_thread_id,
            to_agent_thread_id: request.to_agent_thread_id,
            kind: request.kind,
            payload: request.payload,
            causation_id: request.causation_id,
            created_at: Utc::now(),
            delivered_at: None,
            acknowledged_at: None,
        };
        if let Some(causation_id) = message.causation_id {
            state.deduplication.insert(
                (
                    message.session_id,
                    message.to_agent_thread_id,
                    message.kind,
                    causation_id,
                ),
                message.id,
            );
        }
        state.messages.push(message.clone());
        Ok(message)
    }

    async fn snapshot(
        &self,
        session_id: CollaborationSessionId,
        target: AgentThreadId,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<AgentMailboxMessage>, AgentMailboxError> {
        let mut state = self.state.lock().map_err(|_| AgentMailboxError::Poisoned)?;
        let mut messages = state
            .messages
            .iter_mut()
            .filter(|message| {
                message.session_id == session_id
                    && message.to_agent_thread_id == target
                    && message.acknowledged_at.is_none()
                    && after_sequence.is_none_or(|sequence| message.sequence > sequence)
            })
            .take(limit.clamp(1, 256))
            .map(|message| {
                message.delivered_at.get_or_insert_with(Utc::now);
                message.clone()
            })
            .collect::<Vec<_>>();
        messages.sort_by_key(|message| message.sequence);
        Ok(messages)
    }

    async fn acknowledge(
        &self,
        target: AgentThreadId,
        message_ids: &[AgentMailboxMessageId],
    ) -> Result<(), AgentMailboxError> {
        let mut state = self.state.lock().map_err(|_| AgentMailboxError::Poisoned)?;
        for message_id in message_ids {
            let message = state
                .messages
                .iter_mut()
                .find(|message| message.id == *message_id)
                .ok_or(AgentMailboxError::MessageNotFound(*message_id))?;
            if message.to_agent_thread_id != target {
                return Err(AgentMailboxError::WrongTarget {
                    message_id: *message_id,
                    target,
                });
            }
            message.acknowledged_at.get_or_insert_with(Utc::now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(
        session_id: CollaborationSessionId,
        from: AgentThreadId,
        to: AgentThreadId,
        causation_id: Option<Uuid>,
    ) -> EnqueueAgentMessage {
        EnqueueAgentMessage {
            session_id,
            from_agent_thread_id: from,
            to_agent_thread_id: to,
            kind: AgentMailboxMessageKind::Message,
            payload: json!({ "text": "hello" }),
            causation_id,
        }
    }

    #[tokio::test]
    async fn mailbox_sequences_messages_per_session_and_acknowledges_them() {
        let mailbox = InMemoryAgentMailbox::new();
        let session_id = CollaborationSessionId::new();
        let from = AgentThreadId::new();
        let to = AgentThreadId::new();
        let first = mailbox
            .enqueue(request(session_id, from, to, None))
            .await
            .unwrap();
        let second = mailbox
            .enqueue(request(session_id, from, to, None))
            .await
            .unwrap();
        assert_eq!((first.sequence, second.sequence), (1, 2));

        mailbox.acknowledge(to, &[first.id]).await.unwrap();
        let pending = mailbox.snapshot(session_id, to, None, 10).await.unwrap();
        assert_eq!(
            pending.iter().map(|message| message.id).collect::<Vec<_>>(),
            vec![second.id]
        );
    }

    #[tokio::test]
    async fn causation_id_makes_delivery_idempotent() {
        let mailbox = InMemoryAgentMailbox::new();
        let session_id = CollaborationSessionId::new();
        let from = AgentThreadId::new();
        let to = AgentThreadId::new();
        let causation_id = Uuid::new_v4();
        let first = mailbox
            .enqueue(request(session_id, from, to, Some(causation_id)))
            .await
            .unwrap();
        let duplicate = mailbox
            .enqueue(request(session_id, from, to, Some(causation_id)))
            .await
            .unwrap();
        assert_eq!(first.id, duplicate.id);
        assert_eq!(first.sequence, duplicate.sequence);
    }
}
