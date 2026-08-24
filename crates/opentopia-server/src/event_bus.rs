use opentopia_core::{AgentEvent, AgentEventPayload};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct EventBus {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<AgentEvent>>>>,
    activity: broadcast::Sender<AgentEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        let (activity, _rx) = broadcast::channel(1024);
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
            activity,
        }
    }
}

impl EventBus {
    pub(super) fn subscribe(&self, thread_id: Uuid) -> broadcast::Receiver<AgentEvent> {
        let mut channels = self.channels.write().expect("event bus poisoned");
        channels
            .entry(thread_id)
            .or_insert_with(|| {
                let (tx, _rx) = broadcast::channel(256);
                tx
            })
            .subscribe()
    }

    pub(super) fn subscribe_activity(&self) -> broadcast::Receiver<AgentEvent> {
        self.activity.subscribe()
    }

    pub(super) fn publish(&self, event: AgentEvent) {
        if is_thread_activity_event(&event) {
            let _ = self.activity.send(event.clone());
        }
        let sender = {
            let mut channels = self.channels.write().expect("event bus poisoned");
            channels
                .entry(event.thread_id)
                .or_insert_with(|| {
                    let (tx, _rx) = broadcast::channel(256);
                    tx
                })
                .clone()
        };
        let _ = sender.send(event);
    }
}

fn is_thread_activity_event(event: &AgentEvent) -> bool {
    matches!(
        &event.payload,
        AgentEventPayload::TurnStarted { .. }
            | AgentEventPayload::ApprovalRequested { .. }
            | AgentEventPayload::TurnSuspended { .. }
            | AgentEventPayload::BrowserHandoffRequired { .. }
            | AgentEventPayload::UserInputRequested { .. }
            | AgentEventPayload::TurnAwaitingInput { .. }
            | AgentEventPayload::TurnFinished { .. }
            | AgentEventPayload::TurnCancelled { .. }
            | AgentEventPayload::Error { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_activity_channel_excludes_high_volume_deltas() {
        let bus = EventBus::default();
        let mut activity = bus.subscribe_activity();
        let thread_id = Uuid::new_v4();

        bus.publish(AgentEvent::new(
            thread_id,
            Some(Uuid::new_v4()),
            1,
            AgentEventPayload::ModelDelta {
                text: "token".to_string(),
            },
        ));
        assert!(matches!(
            activity.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        bus.publish(AgentEvent::new(
            thread_id,
            Some(Uuid::new_v4()),
            2,
            AgentEventPayload::TurnFinished {
                summary: "done".to_string(),
            },
        ));
        assert!(matches!(
            activity.try_recv().expect("activity event").payload,
            AgentEventPayload::TurnFinished { .. }
        ));
    }
}
