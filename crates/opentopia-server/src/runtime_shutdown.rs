use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// One-way process lifecycle gate. Once shutdown preparation begins, no new
/// product Turn may start in this server process.
#[derive(Clone, Default)]
pub(super) struct RuntimeShutdown {
    preparing: Arc<AtomicBool>,
}

impl RuntimeShutdown {
    pub(super) fn begin(&self) {
        self.preparing.store(true, Ordering::Release);
    }

    pub(super) fn is_preparing(&self) -> bool {
        self.preparing.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_gate_is_shared_and_one_way() {
        let shutdown = RuntimeShutdown::default();
        let observer = shutdown.clone();
        assert!(!observer.is_preparing());
        shutdown.begin();
        assert!(observer.is_preparing());
    }
}
