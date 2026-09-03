use crate::model::{AgentEventPayload, ProviderDeltaAttempt};
use crate::tool_error::ensure_tool_error_record;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
/// Provider streams commonly split text into only a handful of characters per
/// delta. Persisting each fragment as its own durable event amplifies one model
/// response into thousands of SQLite transactions, so adjacent fragments are
/// folded into bounded chunks before they reach the event sink.
const STREAM_EVENT_COALESCE_BYTES: usize = 8 * 1024;
const STREAM_EVENT_COALESCE_INTERVAL: Duration = Duration::from_millis(100);

pub type AgentEventSender = mpsc::UnboundedSender<AgentEventPayload>;

pub(super) struct TurnEvents {
    items: Vec<AgentEventPayload>,
    pub(super) sender: Option<AgentEventSender>,
    pending_stream: Option<PendingStreamEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamEventKind {
    Model,
    Reasoning,
}

struct PendingStreamEvent {
    kind: StreamEventKind,
    text: String,
    provider_attempt: Option<ProviderDeltaAttempt>,
    started_at: Instant,
}

impl TurnEvents {
    pub(super) fn new(sender: Option<AgentEventSender>) -> Self {
        Self {
            items: Vec::new(),
            sender,
            pending_stream: None,
        }
    }

    pub(super) fn from_recorded(items: Vec<AgentEventPayload>) -> Self {
        Self {
            items,
            sender: None,
            pending_stream: None,
        }
    }

    pub(super) fn items(&self) -> &[AgentEventPayload] {
        &self.items
    }

    pub(super) fn push(&mut self, payload: AgentEventPayload) {
        match payload {
            AgentEventPayload::ModelDelta {
                text,
                provider_attempt,
            } => {
                self.push_stream_delta(StreamEventKind::Model, text, provider_attempt);
            }
            AgentEventPayload::ReasoningDelta {
                text,
                provider_attempt,
            } => {
                self.push_stream_delta(StreamEventKind::Reasoning, text, provider_attempt);
            }
            payload => {
                self.flush_pending_stream();
                self.push_immediate(payload, true);
            }
        }
    }

    fn push_immediate(&mut self, mut payload: AgentEventPayload, publish: bool) {
        if let AgentEventPayload::ToolCallFinished { result } = &mut payload {
            ensure_tool_error_record(result);
        }
        if publish {
            if let Some(sender) = &self.sender {
                // Strip the potentially giant transcript before cloning the
                // durable event, then move the sole checkpoint copy into the
                // live sink.
                let checkpoint = payload.take_provider_request_checkpoint();
                let durable_payload = payload.clone();
                payload.set_provider_request_checkpoint(checkpoint);
                let _ = sender.send(payload);
                self.items.push(durable_payload);
                return;
            }
        }
        // Provider request checkpoints can contain the entire prompt. They are
        // delivered once through the live sink and must not be retained in the
        // Turn result or duplicated in the append-only event log.
        payload.take_provider_request_checkpoint();
        self.items.push(payload);
    }

    fn push_stream_delta(
        &mut self,
        kind: StreamEventKind,
        text: String,
        provider_attempt: Option<ProviderDeltaAttempt>,
    ) {
        if text.is_empty() {
            return;
        }

        if self.pending_stream.as_ref().is_some_and(|pending| {
            pending.kind != kind || pending.provider_attempt != provider_attempt
        }) {
            self.flush_pending_stream();
        }

        let pending = self
            .pending_stream
            .get_or_insert_with(|| PendingStreamEvent {
                kind,
                text: String::new(),
                provider_attempt,
                started_at: Instant::now(),
            });
        pending.text.push_str(&text);
        if pending.text.len() >= STREAM_EVENT_COALESCE_BYTES
            || pending.started_at.elapsed() >= STREAM_EVENT_COALESCE_INTERVAL
        {
            self.flush_pending_stream();
        }
    }

    fn flush_pending_stream(&mut self) {
        let Some(pending) = self.pending_stream.take() else {
            return;
        };
        let payload = match pending.kind {
            StreamEventKind::Model => AgentEventPayload::ModelDelta {
                text: pending.text,
                provider_attempt: pending.provider_attempt,
            },
            StreamEventKind::Reasoning => AgentEventPayload::ReasoningDelta {
                text: pending.text,
                provider_attempt: pending.provider_attempt,
            },
        };
        self.push_immediate(payload, true);
    }

    pub(super) fn record(&mut self, payload: AgentEventPayload) {
        self.push_immediate(payload, false);
    }

    pub(super) fn into_vec(mut self) -> Vec<AgentEventPayload> {
        self.flush_pending_stream();
        self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ProviderRequestCheckpoint, ProviderWireTranscript};
    use serde_json::{json, Value};
    use uuid::Uuid;

    #[test]
    fn provider_request_checkpoint_is_delivered_live_but_not_retained() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let request_id = Uuid::new_v4();
        let checkpoint = ProviderRequestCheckpoint {
            compatibility_hash: "compat-1".to_string(),
            transcript: ProviderWireTranscript {
                format: "openai_chat_native_messages_v1".to_string(),
                items: vec![json!({"role": "user", "content": "sent request"})],
            },
        };
        let mut events = TurnEvents::new(Some(sender));

        events.push(AgentEventPayload::ProviderRequestSent {
            request_id,
            round: 1,
            attempt: 1,
            adapter: "openai_chat".to_string(),
            method: "POST".to_string(),
            endpoint: "https://example.invalid/v1/chat/completions".to_string(),
            cache_trace: None,
            body: Value::Null,
            checkpoint: Some(checkpoint.clone()),
        });

        let mut live = receiver.try_recv().expect("live request event");
        assert_eq!(live.take_provider_request_checkpoint(), Some(checkpoint));
        let mut recorded = events.into_vec();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].take_provider_request_checkpoint().is_none());
        assert!(serde_json::to_value(&recorded[0])
            .expect("serialize event")
            .get("checkpoint")
            .is_none());
    }

    #[test]
    fn stream_coalescing_preserves_provider_attempt_boundaries() {
        let request_id = Uuid::new_v4();
        let mut events = TurnEvents::new(None);

        for (text, attempt) in [("old", 1), ("new", 2)] {
            events.push(AgentEventPayload::ModelDelta {
                text: text.to_string(),
                provider_attempt: Some(ProviderDeltaAttempt {
                    request_id,
                    round: 1,
                    attempt,
                }),
            });
        }

        let events = events.into_vec();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            AgentEventPayload::ModelDelta {
                text,
                provider_attempt: Some(origin),
            } if text == "old" && origin.attempt == 1
        ));
        assert!(matches!(
            &events[1],
            AgentEventPayload::ModelDelta {
                text,
                provider_attempt: Some(origin),
            } if text == "new" && origin.attempt == 2
        ));
    }
}
