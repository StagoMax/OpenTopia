use opentopia_core::ProviderAdapterError;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

pub(super) const QUOTA_EXHAUSTED_MESSAGE: &str =
    "额度不足：当前 Provider 账户没有可用额度。请充值或切换 Provider；充值后请在设置中重新测试连接。";

/// Process-scoped circuit state for failures that will not recover by retrying
/// the same model request. A successful connection test or a provider settings
/// change is the recovery boundary.
#[derive(Clone, Default)]
pub(super) struct ProviderRuntimeHealth {
    quota_failures: Arc<RwLock<HashSet<String>>>,
}

impl ProviderRuntimeHealth {
    pub(super) fn block_for_quota(&self, provider_id: &str) {
        self.quota_failures
            .write()
            .expect("provider runtime health lock poisoned")
            .insert(provider_id.to_string());
    }

    pub(super) fn blocked_message(&self, provider_id: &str) -> Option<&'static str> {
        self.quota_failures
            .read()
            .expect("provider runtime health lock poisoned")
            .contains(provider_id)
            .then_some(QUOTA_EXHAUSTED_MESSAGE)
    }

    pub(super) fn clear(&self, provider_id: &str) {
        self.quota_failures
            .write()
            .expect("provider runtime health lock poisoned")
            .remove(provider_id);
    }

    pub(super) fn clear_all(&self) {
        self.quota_failures
            .write()
            .expect("provider runtime health lock poisoned")
            .clear();
    }
}

pub(super) fn provider_failure_is_quota_exhausted(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<ProviderAdapterError>()
            .is_some_and(|error| matches!(error, ProviderAdapterError::QuotaExhausted { .. }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_block_requires_an_explicit_recovery_boundary() {
        let health = ProviderRuntimeHealth::default();
        health.block_for_quota("provider-1");
        assert_eq!(
            health.blocked_message("provider-1"),
            Some(QUOTA_EXHAUSTED_MESSAGE)
        );
        health.clear("provider-1");
        assert_eq!(health.blocked_message("provider-1"), None);
    }

    #[test]
    fn recognizes_a_context_wrapped_quota_failure() {
        let error = anyhow::Error::new(ProviderAdapterError::QuotaExhausted {
            detail: "insufficient_user_quota".to_string(),
        })
        .context("model round failed");
        assert!(provider_failure_is_quota_exhausted(&error));
    }
}
