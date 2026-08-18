use crate::model::AgentEventPayload;
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
            AgentEventPayload::ModelDelta { text } => {
                self.push_stream_delta(StreamEventKind::Model, text);
            }
            AgentEventPayload::ReasoningDelta { text } => {
                self.push_stream_delta(StreamEventKind::Reasoning, text);
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
                let _ = sender.send(payload.clone());
            }
        }
        self.items.push(payload);
    }

    fn push_stream_delta(&mut self, kind: StreamEventKind, text: String) {
        if text.is_empty() {
            return;
        }

        if self
            .pending_stream
            .as_ref()
            .is_some_and(|pending| pending.kind != kind)
        {
            self.flush_pending_stream();
        }

        let pending = self
            .pending_stream
            .get_or_insert_with(|| PendingStreamEvent {
                kind,
                text: String::new(),
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
            StreamEventKind::Model => AgentEventPayload::ModelDelta { text: pending.text },
            StreamEventKind::Reasoning => AgentEventPayload::ReasoningDelta { text: pending.text },
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
