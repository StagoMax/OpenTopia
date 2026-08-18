use crate::browser::{
    BrowserAction, BrowserActionReceipt, BrowserDownloadRequest, BrowserError,
    BrowserNavigateRequest, BrowserNetworkGrant, BrowserNode, BrowserNodeRef, BrowserObservation,
    BrowserObservationId, BrowserObserveOptions, BrowserOutput, BrowserRuntime,
    BrowserRuntimeCapabilities, BrowserSessionId, BrowserSessionInfo, BrowserSessionSpec,
    BrowserTargetRef, BrowserWaitRequest,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserRuntimeRoute {
    Managed,
    Chrome,
}

#[derive(Clone)]
pub struct BrowserRuntimeRouter {
    managed: Arc<dyn BrowserRuntime>,
    chrome: Option<Arc<dyn BrowserRuntime>>,
    bindings: Arc<Mutex<HashMap<BrowserSessionId, BrowserRuntimeRoute>>>,
    session_gates: Arc<Mutex<HashMap<BrowserSessionId, Arc<Mutex<()>>>>>,
}

impl BrowserRuntimeRouter {
    pub fn new(managed: Arc<dyn BrowserRuntime>, chrome: Option<Arc<dyn BrowserRuntime>>) -> Self {
        Self {
            managed,
            chrome,
            bindings: Arc::new(Mutex::new(HashMap::new())),
            session_gates: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn chrome_available(&self) -> bool {
        self.chrome.is_some()
    }

    pub async fn route_for(&self, session: BrowserSessionId) -> BrowserRuntimeRoute {
        self.bindings
            .lock()
            .await
            .get(&session)
            .copied()
            .unwrap_or(BrowserRuntimeRoute::Managed)
    }

    pub async fn bind(
        &self,
        spec: BrowserSessionSpec,
        route: BrowserRuntimeRoute,
    ) -> Result<BrowserSessionInfo, BrowserError> {
        let gate = self.session_gate(spec.session_id).await;
        let _guard = gate.lock().await;
        // Resolve the destination before disturbing the active route. In
        // particular, an unavailable Chrome bridge must not tear down a
        // healthy managed session.
        let runtime = self.runtime(route)?.clone();
        let current = self.route_for(spec.session_id).await;
        let previous = if current != route {
            let previous = self.runtime(current)?.clone();
            if let Err(error) = previous.close_session(spec.session_id).await {
                if !matches!(error, BrowserError::SessionNotFound(_)) {
                    return Err(error);
                }
            }
            Some(previous)
        } else {
            None
        };
        let info = match runtime.create_session(spec.clone()).await {
            Ok(info) => info,
            Err(error) => {
                if let Some(previous) = previous {
                    let _ = previous.create_session(spec).await;
                }
                return Err(error);
            }
        };
        self.bindings.lock().await.insert(spec.session_id, route);
        Ok(info)
    }

    fn runtime(
        &self,
        route: BrowserRuntimeRoute,
    ) -> Result<&Arc<dyn BrowserRuntime>, BrowserError> {
        match route {
            BrowserRuntimeRoute::Managed => Ok(&self.managed),
            BrowserRuntimeRoute::Chrome => self.chrome.as_ref().ok_or_else(|| {
                BrowserError::BrokerConfiguration("Chrome bridge is unavailable".to_string())
            }),
        }
    }

    async fn runtime_for(
        &self,
        session: BrowserSessionId,
    ) -> Result<&Arc<dyn BrowserRuntime>, BrowserError> {
        self.runtime(self.route_for(session).await)
    }

    async fn session_gate(&self, session: BrowserSessionId) -> Arc<Mutex<()>> {
        self.session_gates
            .lock()
            .await
            .entry(session)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn locked_runtime(
        &self,
        session: BrowserSessionId,
    ) -> Result<(OwnedMutexGuard<()>, Arc<dyn BrowserRuntime>), BrowserError> {
        let guard = self.session_gate(session).await.lock_owned().await;
        let runtime = self.runtime_for(session).await?.clone();
        Ok((guard, runtime))
    }
}

#[async_trait]
impl BrowserRuntime for BrowserRuntimeRouter {
    fn capabilities(&self) -> BrowserRuntimeCapabilities {
        self.managed.capabilities()
    }

    async fn create_session(
        &self,
        spec: BrowserSessionSpec,
    ) -> Result<BrowserSessionInfo, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(spec.session_id).await?;
        runtime.create_session(spec).await
    }

    async fn grant_network_access(
        &self,
        session: BrowserSessionId,
        grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.grant_network_access(session, grant).await
    }

    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.navigate(session, request).await
    }

    async fn observe(
        &self,
        session: BrowserSessionId,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.observe(session, options).await
    }

    async fn switch_target(
        &self,
        session: BrowserSessionId,
        target: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.switch_target(session, target).await
    }

    async fn observation_node(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime
            .observation_node(session, observation_id, node_ref)
            .await
    }

    async fn perform(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
        action: BrowserAction,
    ) -> Result<BrowserActionReceipt, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime
            .perform(session, observation_id, node_ref, action)
            .await
    }

    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.screenshot(session).await
    }

    async fn wait(
        &self,
        session: BrowserSessionId,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.wait(session, request).await
    }

    async fn download(
        &self,
        session: BrowserSessionId,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let (_guard, runtime) = self.locked_runtime(session).await?;
        runtime.download(session, request).await
    }

    async fn close_session(&self, session: BrowserSessionId) -> Result<(), BrowserError> {
        let gate = self.session_gate(session).await;
        let guard = gate.clone().lock_owned().await;
        let route = self
            .bindings
            .lock()
            .await
            .get(&session)
            .copied()
            .unwrap_or(BrowserRuntimeRoute::Managed);
        let result = self.runtime(route)?.close_session(session).await;
        if result.is_ok() || matches!(&result, Err(BrowserError::SessionNotFound(_))) {
            self.bindings.lock().await.remove(&session);
        }
        drop(guard);
        let mut gates = self.session_gates.lock().await;
        if gates
            .get(&session)
            .is_some_and(|current| Arc::ptr_eq(current, &gate) && Arc::strong_count(current) == 2)
        {
            gates.remove(&session);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{BrowserRuntimeConfig, LocalBrowserRuntime};

    #[tokio::test]
    async fn unavailable_chrome_route_keeps_the_managed_binding() {
        let managed: Arc<dyn BrowserRuntime> =
            Arc::new(LocalBrowserRuntime::new(BrowserRuntimeConfig::default()));
        let router = BrowserRuntimeRouter::new(managed, None);
        let session = BrowserSessionId::new();

        let error = router
            .bind(
                BrowserSessionSpec::from(session),
                BrowserRuntimeRoute::Chrome,
            )
            .await
            .expect_err("Chrome must not bind without a configured bridge");

        assert!(matches!(error, BrowserError::BrokerConfiguration(_)));
        assert_eq!(
            router.route_for(session).await,
            BrowserRuntimeRoute::Managed
        );
    }
}
