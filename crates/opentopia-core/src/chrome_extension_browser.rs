use crate::browser::{
    BrowserAction, BrowserActionCapability, BrowserActionReceipt, BrowserBackendKind,
    BrowserDownloadRequest, BrowserError, BrowserNavigateRequest, BrowserNetworkGrant, BrowserNode,
    BrowserNodeRef, BrowserObservation, BrowserObservationId, BrowserObserveOptions, BrowserOutput,
    BrowserProfilePersistence, BrowserRuntime, BrowserRuntimeCapabilities, BrowserRuntimeConfig,
    BrowserSessionId, BrowserSessionInfo, BrowserSessionSpec, BrowserSurfaceKind, BrowserTargetRef,
    BrowserWaitRequest, LocalBrowserSession,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct ChromeExtensionBrowserRuntimeConfig {
    pub bridge_url: String,
    pub bridge_token: String,
    pub browser: BrowserRuntimeConfig,
}

#[derive(Clone)]
pub struct ChromeExtensionBrowserRuntime {
    config: Arc<ChromeExtensionBrowserRuntimeConfig>,
    sessions: Arc<Mutex<HashMap<BrowserSessionId, Arc<Mutex<LocalBrowserSession>>>>>,
    specs: Arc<Mutex<HashMap<BrowserSessionId, BrowserSessionSpec>>>,
}

impl ChromeExtensionBrowserRuntime {
    pub fn new(config: ChromeExtensionBrowserRuntimeConfig) -> Result<Self, BrowserError> {
        if config.bridge_url.trim().is_empty() || config.bridge_token.trim().is_empty() {
            return Err(BrowserError::BrokerConfiguration(
                "Chrome bridge URL and token are required".to_string(),
            ));
        }
        Ok(Self {
            config: Arc::new(config),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            specs: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn health_check(&self) -> Result<(), BrowserError> {
        let endpoint = format!(
            "{}/v1/backend/health",
            self.config.bridge_url.trim_end_matches('/')
        );
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()?
            .get(endpoint)
            .bearer_auth(self.config.bridge_token.trim())
            .send()
            .await
            .map_err(|_| BrowserError::BrokerUnavailable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(BrowserError::BrokerRejected {
                status: response.status().as_u16(),
                message: "Chrome bridge health check was rejected".to_string(),
            })
        }
    }

    async fn session(
        &self,
        session_id: BrowserSessionId,
    ) -> Result<Arc<Mutex<LocalBrowserSession>>, BrowserError> {
        self.sessions
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or(BrowserError::SessionNotFound(session_id))
    }

    fn validate_url(raw: &str) -> Result<(), BrowserError> {
        let url =
            reqwest::Url::parse(raw).map_err(|_| BrowserError::InvalidUrl(raw.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(BrowserError::DisallowedScheme(url.scheme().to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for ChromeExtensionBrowserRuntime {
    fn capabilities(&self) -> BrowserRuntimeCapabilities {
        BrowserRuntimeCapabilities {
            protocol_version: 1,
            backend: BrowserBackendKind::ChromeExtension,
            surface: BrowserSurfaceKind::ExternalWindow,
            profile_persistence: vec![BrowserProfilePersistence::Persistent],
            actions: vec![
                BrowserActionCapability::Navigate,
                BrowserActionCapability::Observe,
                BrowserActionCapability::SwitchTarget,
                BrowserActionCapability::Click,
                BrowserActionCapability::Type,
                BrowserActionCapability::Select,
                BrowserActionCapability::Hover,
                BrowserActionCapability::Scroll,
                BrowserActionCapability::Screenshot,
                BrowserActionCapability::Wait,
            ],
            hard_network_isolation: false,
            supports_user_handoff: true,
            supports_external_profile: true,
        }
    }

    async fn create_session(
        &self,
        spec: BrowserSessionSpec,
    ) -> Result<BrowserSessionInfo, BrowserError> {
        if spec.profile_persistence != BrowserProfilePersistence::Persistent {
            return Err(BrowserError::BrokerConfiguration(
                "Attached Chrome profiles are persistent and cannot be ephemeral".to_string(),
            ));
        }
        {
            let specs = self.specs.lock().await;
            if let Some(existing) = specs.get(&spec.session_id) {
                if existing != &spec {
                    return Err(BrowserError::SessionProfileConflict {
                        session: spec.session_id,
                    });
                }
                if self.sessions.lock().await.contains_key(&spec.session_id) {
                    return Ok(BrowserSessionInfo {
                        session_id: spec.session_id,
                        profile_id: spec.profile_id,
                        profile_persistence: spec.profile_persistence,
                        backend: BrowserBackendKind::ChromeExtension,
                    });
                }
            }
        }
        let runtime = LocalBrowserSession::start_external(
            Arc::new(self.config.browser.clone()),
            &self.config.bridge_url,
            &self.config.bridge_token,
            spec.session_id,
        )
        .await?;
        self.sessions
            .lock()
            .await
            .insert(spec.session_id, Arc::new(Mutex::new(runtime)));
        self.specs
            .lock()
            .await
            .insert(spec.session_id, spec.clone());
        Ok(BrowserSessionInfo {
            session_id: spec.session_id,
            profile_id: spec.profile_id,
            profile_persistence: spec.profile_persistence,
            backend: BrowserBackendKind::ChromeExtension,
        })
    }

    async fn grant_network_access(
        &self,
        session: BrowserSessionId,
        _grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError> {
        self.session(session).await?;
        // The attached personal tab intentionally has no hard request interception.
        Ok(())
    }

    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        Self::validate_url(&request.url)?;
        self.session(session)
            .await?
            .lock()
            .await
            .navigate(request)
            .await
    }

    async fn observe(
        &self,
        session: BrowserSessionId,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError> {
        self.session(session)
            .await?
            .lock()
            .await
            .observe(options)
            .await
    }

    async fn switch_target(
        &self,
        session: BrowserSessionId,
        target: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError> {
        self.session(session)
            .await?
            .lock()
            .await
            .switch_target(target)
            .await
    }

    async fn observation_node(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError> {
        self.session(session)
            .await?
            .lock()
            .await
            .observation_node(observation_id, node_ref)
            .await
    }

    async fn perform(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
        action: BrowserAction,
    ) -> Result<BrowserActionReceipt, BrowserError> {
        self.session(session)
            .await?
            .lock()
            .await
            .perform(observation_id, node_ref, action)
            .await
    }

    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        self.session(session).await?.lock().await.screenshot().await
    }

    async fn wait(
        &self,
        session: BrowserSessionId,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.session(session)
            .await?
            .lock()
            .await
            .wait(request)
            .await
    }

    async fn download(
        &self,
        session: BrowserSessionId,
        _request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.session(session).await?;
        Err(BrowserError::Protocol(
            "Downloads are not supported for an attached personal Chrome tab".to_string(),
        ))
    }

    async fn close_session(&self, session: BrowserSessionId) -> Result<(), BrowserError> {
        let runtime = self
            .sessions
            .lock()
            .await
            .remove(&session)
            .ok_or(BrowserError::SessionNotFound(session))?;
        self.specs.lock().await.remove(&session);
        let result = runtime.lock().await.shutdown().await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_external_profile_capabilities_honestly() {
        let runtime = ChromeExtensionBrowserRuntime::new(ChromeExtensionBrowserRuntimeConfig {
            bridge_url: "http://127.0.0.1:32191".to_string(),
            bridge_token: "test-token".to_string(),
            browser: BrowserRuntimeConfig::default(),
        })
        .unwrap();
        let capabilities = runtime.capabilities();
        assert_eq!(capabilities.backend, BrowserBackendKind::ChromeExtension);
        assert_eq!(capabilities.surface, BrowserSurfaceKind::ExternalWindow);
        assert!(!capabilities.hard_network_isolation);
        assert!(capabilities.supports_external_profile);
        assert!(!capabilities
            .actions
            .contains(&BrowserActionCapability::Download));
    }
}
