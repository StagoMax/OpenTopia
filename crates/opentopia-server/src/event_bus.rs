use opentopia_core::AgentEvent;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone, Default)]
pub(super) struct EventBus {
    channels: Arc<RwLock<HashMap<Uuid, broadcast::Sender<AgentEvent>>>>,
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

    pub(super) fn publish(&self, event: AgentEvent) {
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
