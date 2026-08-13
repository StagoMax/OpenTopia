use crate::browser::{
    BrowserAction, BrowserActionReceipt, BrowserDownloadRequest, BrowserError,
    BrowserNavigateRequest, BrowserNetworkGrant, BrowserNode, BrowserNodeRef, BrowserObservation,
    BrowserObservationId, BrowserObserveOptions, BrowserOutput, BrowserRuntime,
    BrowserRuntimeCapabilities, BrowserSessionId, BrowserSessionInfo, BrowserSessionSpec,
    BrowserTargetRef, BrowserWaitRequest,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl BrowserRuntimeRouter {
    pub fn new(managed: Arc<dyn BrowserRuntime>, chrome: Option<Arc<dyn BrowserRuntime>>) -> Self {
        Self {
            managed,
            chrome,
            bindings: Arc::new(Mutex::new(HashMap::new())),
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
        let current = self.route_for(spec.session_id).await;
        if current != route {
            let previous = self.runtime(current)?;
            if let Err(error) = previous.close_session(spec.session_id).await {
                if !matches!(error, BrowserError::SessionNotFound(_)) {
                    return Err(error);
                }
            }
        }
        let runtime = self.runtime(route)?;
        let info = runtime.create_session(spec.clone()).await?;
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
        self.runtime_for(spec.session_id)
            .await?
            .create_session(spec)
            .await
    }

    async fn grant_network_access(
        &self,
        session: BrowserSessionId,
        grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError> {
        self.runtime_for(session)
            .await?
            .grant_network_access(session, grant)
            .await
    }

    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.runtime_for(session)
            .await?
            .navigate(session, request)
            .await
    }

    async fn observe(
        &self,
        session: BrowserSessionId,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError> {
        self.runtime_for(session)
            .await?
            .observe(session, options)
            .await
    }

    async fn switch_target(
        &self,
        session: BrowserSessionId,
        target: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError> {
        self.runtime_for(session)
            .await?
            .switch_target(session, target)
            .await
    }

    async fn observation_node(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError> {
        self.runtime_for(session)
            .await?
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
        self.runtime_for(session)
            .await?
            .perform(session, observation_id, node_ref, action)
            .await
    }

    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        self.runtime_for(session).await?.screenshot(session).await
    }

    async fn wait(
        &self,
        session: BrowserSessionId,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.runtime_for(session)
            .await?
            .wait(session, request)
            .await
    }

    async fn download(
        &self,
        session: BrowserSessionId,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.runtime_for(session)
            .await?
            .download(session, request)
            .await
    }

    async fn close_session(&self, session: BrowserSessionId) -> Result<(), BrowserError> {
        let route = self
            .bindings
            .lock()
            .await
            .remove(&session)
            .unwrap_or(BrowserRuntimeRoute::Managed);
        self.runtime(route)?.close_session(session).await
    }
}
