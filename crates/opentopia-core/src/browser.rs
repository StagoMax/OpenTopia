//! A bounded Chromium runtime with one shared browser profile and task-scoped target groups.
//!
//! This module intentionally stops at the browser boundary: callers decide whether a URL or an
//! interaction needs approval, while this runtime owns the browser process and its per-session
//! profile. The `BrowserContent` enum is a richer result contract than the current text-only tool
//! result and can be adapted to a future multimodal message protocol without re-reading data.

mod cdp_transport;
mod contract;
mod io_support;
mod process;

pub(crate) use cdp_transport::validate_chrome_bridge_url;
use cdp_transport::CdpPage;
use contract::DEFAULT_BROWSER_PROFILE_ID;
pub use contract::*;
use io_support::{list_downloads, png_looks_blank, wait_for_download};
use process::{browser_profile_storage_root, LocalBrowserProcess};

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_SCREENSHOT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_NETWORK_HOSTS: usize = 256;
const OBSERVATION_TTL: Duration = Duration::from_secs(120);
const MAX_OBSERVATIONS_PER_SESSION: usize = 12;
const MAX_NODE_POSITION_DRIFT: f64 = 24.0;
const MAX_CHROME_BRIDGE_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Uses the Chrome DevTools Protocol against a locally spawned Chrome or Edge process. CDP is the
/// supported automation protocol for both browsers and lets OpenTopia use an installed browser
/// rather than shipping a second, unpatched browser binary.
#[derive(Clone)]
pub struct LocalBrowserRuntime {
    config: Arc<BrowserRuntimeConfig>,
    sessions: Arc<Mutex<HashMap<BrowserSessionId, Arc<Mutex<LocalBrowserSession>>>>>,
    session_specs: Arc<Mutex<HashMap<BrowserSessionId, BrowserSessionSpec>>>,
    network_grants: Arc<Mutex<HashMap<BrowserSessionId, BrowserNetworkGrant>>>,
    processes: Arc<Mutex<HashMap<BrowserProfileKey, Arc<Mutex<LocalBrowserProcess>>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BrowserProfileKey {
    profile_id: BrowserProfileId,
    profile_persistence: BrowserProfilePersistence,
}

impl From<&BrowserSessionSpec> for BrowserProfileKey {
    fn from(spec: &BrowserSessionSpec) -> Self {
        Self {
            profile_id: spec.profile_id.clone(),
            profile_persistence: spec.profile_persistence,
        }
    }
}

impl LocalBrowserRuntime {
    pub fn new(config: BrowserRuntimeConfig) -> Self {
        Self {
            config: Arc::new(config),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            session_specs: Arc::new(Mutex::new(HashMap::new())),
            network_grants: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn config(&self) -> &BrowserRuntimeConfig {
        &self.config
    }

    async fn session(
        &self,
        session_id: BrowserSessionId,
    ) -> Result<Arc<Mutex<LocalBrowserSession>>, BrowserError> {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(&session_id) {
            let session = session.clone();
            if session.lock().await.page.is_connected() {
                return Ok(session);
            }
            sessions.remove(&session_id);
        }

        let (spec, inserted_spec) = {
            let mut specs = self.session_specs.lock().await;
            if let Some(spec) = specs.get(&session_id) {
                (spec.clone(), false)
            } else {
                let spec = BrowserSessionSpec::from(session_id);
                specs.insert(session_id, spec.clone());
                (spec, true)
            }
        };
        let started = async {
            let process = self.process(&spec).await?;
            LocalBrowserSession::start(self.config.clone(), process).await
        }
        .await;
        let started = match started {
            Ok(started) => started,
            Err(error) => {
                if inserted_spec {
                    let mut specs = self.session_specs.lock().await;
                    if specs.get(&session_id) == Some(&spec) {
                        specs.remove(&session_id);
                    }
                }
                return Err(error);
            }
        };
        if let Some(grant) = self.network_grants.lock().await.get(&session_id).cloned() {
            started.page.grant_network_access(grant)?;
        }
        let session = Arc::new(Mutex::new(started));
        sessions.insert(session_id, session.clone());
        Ok(session)
    }

    async fn process(
        &self,
        spec: &BrowserSessionSpec,
    ) -> Result<Arc<Mutex<LocalBrowserProcess>>, BrowserError> {
        let profile_key = BrowserProfileKey::from(spec);
        let mut processes = self.processes.lock().await;
        if let Some(existing) = processes.get(&profile_key).cloned() {
            if existing.lock().await.child.try_wait()?.is_none() {
                return Ok(existing);
            }
            processes.remove(&profile_key);
        }
        let started = Arc::new(Mutex::new(
            LocalBrowserProcess::start(self.config.clone(), spec).await?,
        ));
        processes.insert(profile_key, started.clone());
        Ok(started)
    }

    fn validate_url(&self, raw_url: &str) -> Result<(), BrowserError> {
        let url = reqwest::Url::parse(raw_url)
            .map_err(|_| BrowserError::InvalidUrl(raw_url.to_string()))?;
        let scheme = url.scheme().to_ascii_lowercase();
        if !self
            .config
            .allowed_schemes
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&scheme))
        {
            return Err(BrowserError::DisallowedScheme(scheme));
        }
        if url.host_str().is_none() {
            return Err(BrowserError::InvalidUrl(raw_url.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl BrowserRuntime for LocalBrowserRuntime {
    fn capabilities(&self) -> BrowserRuntimeCapabilities {
        BrowserRuntimeCapabilities::managed(
            BrowserBackendKind::LocalChrome,
            if self.config.headless {
                BrowserSurfaceKind::Headless
            } else {
                BrowserSurfaceKind::ExternalWindow
            },
            false,
        )
    }

    async fn create_session(
        &self,
        spec: BrowserSessionSpec,
    ) -> Result<BrowserSessionInfo, BrowserError> {
        let mut inserted = false;
        {
            let mut specs = self.session_specs.lock().await;
            if let Some(existing) = specs.get(&spec.session_id) {
                if existing != &spec {
                    return Err(BrowserError::SessionProfileConflict {
                        session: spec.session_id,
                    });
                }
            } else {
                specs.insert(spec.session_id, spec.clone());
                inserted = true;
            }
        }
        if let Err(error) = self.session(spec.session_id).await {
            if inserted {
                let mut specs = self.session_specs.lock().await;
                if specs.get(&spec.session_id) == Some(&spec) {
                    specs.remove(&spec.session_id);
                }
            }
            return Err(error);
        }
        Ok(BrowserSessionInfo {
            session_id: spec.session_id,
            profile_id: spec.profile_id,
            profile_persistence: spec.profile_persistence,
            backend: BrowserBackendKind::LocalChrome,
        })
    }

    async fn grant_network_access(
        &self,
        session: BrowserSessionId,
        grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError> {
        let runtime = self.session(session).await?;
        let effective_grant = {
            let mut grants = self.network_grants.lock().await;
            let effective_grant = grants.entry(session).or_default();
            effective_grant.merge(grant)?;
            effective_grant.clone()
        };
        let runtime = runtime.lock().await;
        runtime.page.grant_network_access(effective_grant)
    }

    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.validate_url(&request.url)?;
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.navigate(request).await
    }

    async fn observe(
        &self,
        session: BrowserSessionId,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.observe(options).await
    }

    async fn switch_target(
        &self,
        session: BrowserSessionId,
        target: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.switch_target(target).await
    }

    async fn observation_node(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.observation_node(observation_id, node_ref).await
    }

    async fn perform(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
        action: BrowserAction,
    ) -> Result<BrowserActionReceipt, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.perform(observation_id, node_ref, action).await
    }

    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.screenshot().await
    }

    async fn wait(
        &self,
        session: BrowserSessionId,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.wait(request).await
    }

    async fn download(
        &self,
        session: BrowserSessionId,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        self.validate_url(&request.url)?;
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.download(request).await
    }

    async fn close_session(&self, session_id: BrowserSessionId) -> Result<(), BrowserError> {
        let session = self.sessions.lock().await.remove(&session_id);
        let Some(session) = session else {
            return Err(BrowserError::SessionNotFound(session_id));
        };
        let spec = self.session_specs.lock().await.remove(&session_id);
        self.network_grants.lock().await.remove(&session_id);

        {
            let mut session = session.lock().await;
            session.shutdown().await?;
        }
        if let Some(spec) =
            spec.filter(|spec| spec.profile_persistence == BrowserProfilePersistence::Ephemeral)
        {
            let profile_still_used = self.session_specs.lock().await.values().any(|candidate| {
                candidate.profile_id == spec.profile_id
                    && candidate.profile_persistence == spec.profile_persistence
            });
            if !profile_still_used {
                let process = self
                    .processes
                    .lock()
                    .await
                    .remove(&BrowserProfileKey::from(&spec));
                if let Some(process) = process {
                    process.lock().await.shutdown().await?;
                }
                let storage_root = browser_profile_storage_root(&self.config, &spec);
                if storage_root.exists() {
                    tokio::fs::remove_dir_all(storage_root).await?;
                }
            }
        }
        if !self.config.retain_session_data && self.sessions.lock().await.is_empty() {
            let processes = self
                .processes
                .lock()
                .await
                .drain()
                .map(|(_, process)| process)
                .collect::<Vec<_>>();
            for process in processes {
                process.lock().await.shutdown().await?;
            }
            if self.config.data_root.exists() {
                tokio::fs::remove_dir_all(&self.config.data_root).await?;
            }
        }
        Ok(())
    }
}

pub(crate) struct LocalBrowserSession {
    page: CdpPage,
    download_dir: PathBuf,
    command_timeout: Duration,
    max_snapshot_bytes: usize,
    max_screenshot_bytes: usize,
    max_download_bytes: u64,
    observations: HashMap<BrowserObservationId, LocalBrowserObservation>,
    targets: HashMap<String, LocalBrowserTarget>,
    frame_refs: HashMap<(String, String), BrowserFrameRef>,
    dialogs: Vec<BrowserDialog>,
    owns_targets: bool,
    supports_downloads: bool,
    intercepts_network: bool,
}

#[derive(Debug, Clone)]
struct LocalBrowserTarget {
    target_ref: BrowserTargetRef,
    session_id: String,
    opener_target_id: Option<String>,
    url: String,
    title: String,
}

#[derive(Debug, Clone)]
struct LocalBrowserObservation {
    captured_at: Instant,
    url: String,
    target_ref: BrowserTargetRef,
    nodes: HashMap<BrowserNodeRef, LocalBrowserNode>,
}

#[derive(Debug, Clone)]
struct LocalBrowserNode {
    node: BrowserNode,
    locator: LocalNodeLocator,
}

#[derive(Debug, Clone)]
struct LocalNodeLocator {
    target_ref: BrowserTargetRef,
    session_id: String,
    frame_id: String,
    context_id: i64,
    selector_path: Vec<String>,
    node_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserPageState {
    url: String,
    title: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapturedBrowserNode {
    #[serde(default)]
    selector_path: Vec<String>,
    node_key: String,
    role: Option<String>,
    name: Option<String>,
    tag_name: String,
    bounds: BrowserRect,
    href: Option<String>,
    form_action: Option<String>,
    form_method: Option<String>,
    input_type: Option<String>,
    editable: bool,
    #[serde(default)]
    requires_user_action: bool,
    user_action_reason: Option<String>,
    #[serde(skip)]
    locator: Option<LocalNodeLocator>,
    #[serde(skip)]
    frame_ref: Option<BrowserFrameRef>,
}

struct LocalDocumentCapture {
    text: String,
    nodes: Vec<CapturedBrowserNode>,
    frames: Vec<BrowserFrame>,
}

impl LocalBrowserSession {
    async fn start(
        config: Arc<BrowserRuntimeConfig>,
        process: Arc<Mutex<LocalBrowserProcess>>,
    ) -> Result<Self, BrowserError> {
        let (browser_websocket_url, download_dir) = {
            let process = process.lock().await;
            (
                process.browser_websocket_url.clone(),
                process.download_dir.clone(),
            )
        };
        let mut page = CdpPage::connect(&browser_websocket_url, config.command_timeout)
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("connecting to the browser session: {error}"))
            })?;
        page.create_and_attach_target().await.map_err(|error| {
            BrowserError::Protocol(format!("creating the browser target: {error}"))
        })?;
        page.initialize_page_domains().await.map_err(|error| {
            BrowserError::Protocol(format!("initializing browser target domains: {error}"))
        })?;
        page.configure_downloads(&download_dir)
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("subscribing to browser downloads: {error}"))
            })?;
        page.enable_target_discovery().await.map_err(|error| {
            BrowserError::Protocol(format!("enabling popup target discovery: {error}"))
        })?;
        let target_id = page.target_id.clone().ok_or_else(|| {
            BrowserError::Protocol("browser target ID was not retained".to_string())
        })?;
        let session_id = page.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("browser CDP session ID was not retained".to_string())
        })?;
        let target_ref = BrowserTargetRef::new();
        let mut targets = HashMap::new();
        targets.insert(
            target_id,
            LocalBrowserTarget {
                target_ref,
                session_id,
                opener_target_id: None,
                url: "about:blank".to_string(),
                title: String::new(),
            },
        );
        Ok(Self {
            page,
            download_dir,
            command_timeout: config.command_timeout,
            max_snapshot_bytes: config.max_snapshot_bytes,
            max_screenshot_bytes: config.max_screenshot_bytes,
            max_download_bytes: config.max_download_bytes,
            observations: HashMap::new(),
            targets,
            frame_refs: HashMap::new(),
            dialogs: Vec::new(),
            owns_targets: true,
            supports_downloads: true,
            intercepts_network: true,
        })
    }

    pub(crate) async fn start_external(
        config: Arc<BrowserRuntimeConfig>,
        bridge_url: &str,
        bridge_token: &str,
        session_id: BrowserSessionId,
    ) -> Result<Self, BrowserError> {
        let download_dir = config.data_root.join("external-downloads");
        let mut page = CdpPage::connect_chrome_bridge(
            bridge_url,
            bridge_token,
            session_id,
            config.command_timeout,
        )
        .await?;
        page.initialize_external_page_domains().await?;
        page.enable_target_auto_attach().await?;
        let target_id = page.target_id.clone().ok_or_else(|| {
            BrowserError::Protocol("Chrome bridge returned no target ID".to_string())
        })?;
        let target_session_id = page.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("Chrome bridge returned no target session".to_string())
        })?;
        let state = page
            .command(
                "Runtime.evaluate",
                json!({
                    "expression": "({ url: document.location.href, title: document.title })",
                    "returnByValue": true,
                }),
            )
            .await?;
        let value = state.pointer("/result/value").unwrap_or(&Value::Null);
        let mut targets = HashMap::new();
        targets.insert(
            target_id,
            LocalBrowserTarget {
                target_ref: BrowserTargetRef::new(),
                session_id: target_session_id,
                opener_target_id: None,
                url: value
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                title: value
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            },
        );
        Ok(Self {
            page,
            download_dir,
            command_timeout: config.command_timeout,
            max_snapshot_bytes: config.max_snapshot_bytes,
            max_screenshot_bytes: config.max_screenshot_bytes,
            max_download_bytes: config.max_download_bytes,
            observations: HashMap::new(),
            targets,
            frame_refs: HashMap::new(),
            dialogs: Vec::new(),
            owns_targets: false,
            supports_downloads: false,
            intercepts_network: false,
        })
    }

    pub(crate) async fn navigate(
        &mut self,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let previous_url = self.current_url().await?;
        self.page.discard_events();
        let result = self
            .page
            .command("Page.navigate", json!({ "url": &request.url }))
            .await?;
        self.reconcile_navigation(&previous_url, &request.url, &result)
            .await?;
        let wait_error = if let Some(wait) = request.wait {
            match self.wait(wait.clone()).await {
                Ok(_) => None,
                // A page can remain in `interactive` while loading a background resource. The
                // navigation itself is still useful to the model, which can inspect or explicitly
                // wait for a selector rather than losing the whole browser session to a timeout.
                Err(BrowserError::Timeout(_))
                    if matches!(wait.condition, BrowserWaitCondition::DocumentComplete) =>
                {
                    Some("document_complete timed out".to_string())
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        let mut output = self
            .page_output("navigate", json!({ "navigation": result }))
            .await?;
        if let Some(wait_error) = wait_error {
            if let Some(metadata) = output.metadata.as_object_mut() {
                metadata.insert("waitWarning".to_string(), Value::String(wait_error));
            }
        }
        Ok(output)
    }

    async fn reconcile_navigation(
        &mut self,
        previous_url: &str,
        requested_url: &str,
        navigation: &Value,
    ) -> Result<(), BrowserError> {
        let frame_id = navigation.get("frameId").and_then(Value::as_str);
        let loader_id = navigation.get("loaderId").and_then(Value::as_str);
        let reported_error = navigation
            .get("errorText")
            .and_then(Value::as_str)
            .map(str::to_string);
        let deadline = tokio::time::Instant::now() + self.command_timeout;
        let mut committed = false;
        let mut failed_navigation = None;

        loop {
            while let Some(event) = self.page.try_next_event()? {
                match event.method.as_str() {
                    "Page.frameNavigated" => {
                        let frame = event.params.get("frame").unwrap_or(&Value::Null);
                        let event_frame = frame.get("id").and_then(Value::as_str);
                        let event_loader = frame.get("loaderId").and_then(Value::as_str);
                        if frame_id.is_none_or(|expected| event_frame == Some(expected))
                            && loader_id.is_none_or(|expected| event_loader == Some(expected))
                        {
                            committed = true;
                        }
                    }
                    "Page.lifecycleEvent" => {
                        let event_frame = event.params.get("frameId").and_then(Value::as_str);
                        let event_loader = event.params.get("loaderId").and_then(Value::as_str);
                        if frame_id.is_none_or(|expected| event_frame == Some(expected))
                            && loader_id.is_none_or(|expected| event_loader == Some(expected))
                        {
                            committed = true;
                        }
                    }
                    "Page.navigatedWithinDocument" => {
                        let event_frame = event.params.get("frameId").and_then(Value::as_str);
                        if frame_id.is_none_or(|expected| event_frame == Some(expected)) {
                            committed = true;
                        }
                    }
                    "Page.loadEventFired" => committed = true,
                    "Network.loadingFailed" => {
                        if event.params.get("type").and_then(Value::as_str) == Some("Document") {
                            failed_navigation = event
                                .params
                                .get("errorText")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                        }
                    }
                    "OpenTopia.networkRequestBlocked" => {
                        let host = event
                            .params
                            .get("host")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        return Err(BrowserError::NetworkBlocked { host });
                    }
                    _ => {}
                }
            }

            let state = self.navigation_state().await?;
            let current_url = state.get("url").and_then(Value::as_str).unwrap_or_default();
            let ready = state.get("ready").and_then(Value::as_bool).unwrap_or(false);
            let page_changed = current_url != previous_url
                || (!urls_equivalent(previous_url, requested_url)
                    && urls_equivalent(current_url, requested_url));
            if ready && (committed || page_changed) {
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                let message = reported_error
                    .or(failed_navigation)
                    .unwrap_or_else(|| "navigation did not reach a stable document".to_string());
                return Err(BrowserError::Cdp {
                    method: "Page.navigate".to_string(),
                    message,
                });
            }

            tokio::select! {
                event = self.page.next_event() => {
                    if let Some(event) = event? {
                        self.page.push_event(event);
                    }
                }
                _ = tokio::time::sleep(DEFAULT_WAIT_POLL_INTERVAL) => {}
            }
        }
    }

    async fn navigation_state(&mut self) -> Result<Value, BrowserError> {
        self.evaluate_value(
            "({ url: document.location.href, ready: document.readyState !== 'loading' })",
        )
        .await
    }

    fn active_target(&self) -> Result<LocalBrowserTarget, BrowserError> {
        let target_id = self.page.target_id.as_ref().ok_or_else(|| {
            BrowserError::Protocol("No active browser target is attached".to_string())
        })?;
        self.targets.get(target_id).cloned().ok_or_else(|| {
            BrowserError::InvalidTarget("active target is no longer owned".to_string())
        })
    }

    async fn refresh_browser_events(
        &mut self,
        activate_new_popup: bool,
    ) -> Result<(), BrowserError> {
        let mut target_to_activate = None;
        while let Some(event) = self.page.try_next_event()? {
            match event.method.as_str() {
                "Target.attachedToTarget" => {
                    let info = event.params.get("targetInfo").unwrap_or(&Value::Null);
                    if info.get("type").and_then(Value::as_str) != Some("page") {
                        continue;
                    }
                    let Some(target_id) = info.get("targetId").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(session_id) = event.params.get("sessionId").and_then(Value::as_str)
                    else {
                        continue;
                    };
                    if self.targets.contains_key(target_id) {
                        continue;
                    }
                    let opener_target_id = info
                        .get("openerId")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let owned = opener_target_id
                        .as_ref()
                        .is_some_and(|opener| self.targets.contains_key(opener));
                    if !owned {
                        let _ = self
                            .page
                            .root_command(
                                "Target.detachFromTarget",
                                json!({ "sessionId": session_id }),
                            )
                            .await;
                        continue;
                    }
                    if self.intercepts_network {
                        self.page
                            .initialize_page_domains_for_session(session_id)
                            .await?;
                    } else {
                        self.page
                            .initialize_external_page_domains_for_session(session_id)
                            .await?;
                    }
                    self.targets.insert(
                        target_id.to_string(),
                        LocalBrowserTarget {
                            target_ref: BrowserTargetRef::new(),
                            session_id: session_id.to_string(),
                            opener_target_id,
                            url: info
                                .get("url")
                                .and_then(Value::as_str)
                                .unwrap_or("about:blank")
                                .to_string(),
                            title: info
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        },
                    );
                    if activate_new_popup {
                        target_to_activate = Some(target_id.to_string());
                    }
                }
                "Target.targetInfoChanged" => {
                    let info = event.params.get("targetInfo").unwrap_or(&Value::Null);
                    if let Some(target) = info
                        .get("targetId")
                        .and_then(Value::as_str)
                        .and_then(|id| self.targets.get_mut(id))
                    {
                        if let Some(url) = info.get("url").and_then(Value::as_str) {
                            target.url = url.to_string();
                        }
                        if let Some(title) = info.get("title").and_then(Value::as_str) {
                            target.title = title.to_string();
                        }
                    }
                }
                "Target.detachedFromTarget" => {
                    let session_id = event.params.get("sessionId").and_then(Value::as_str);
                    let removed = self
                        .targets
                        .iter()
                        .find(|(_, target)| Some(target.session_id.as_str()) == session_id)
                        .map(|(id, _)| id.clone());
                    if let Some(target_id) = removed {
                        self.targets.remove(&target_id);
                        self.frame_refs.retain(|(owner, _), _| owner != &target_id);
                    }
                }
                "Page.javascriptDialogOpening" => {
                    if let Some(target) = self.targets.values().find(|target| {
                        event.session_id.as_deref() == Some(target.session_id.as_str())
                    }) {
                        self.dialogs.push(BrowserDialog {
                            dialog_type: event
                                .params
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or("dialog")
                                .to_string(),
                            message: event
                                .params
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            default_prompt: event
                                .params
                                .get("defaultPrompt")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            handled: true,
                            target_ref: target.target_ref.clone(),
                        });
                        if self.dialogs.len() > 32 {
                            self.dialogs.remove(0);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(target_id) = target_to_activate {
            self.activate_target_id(&target_id).await?;
        } else if self
            .page
            .target_id
            .as_ref()
            .is_some_and(|active| !self.targets.contains_key(active))
        {
            let fallback = self.targets.keys().next().cloned().ok_or_else(|| {
                BrowserError::InvalidTarget("all owned targets were closed".to_string())
            })?;
            self.activate_target_id(&fallback).await?;
        }
        Ok(())
    }

    async fn activate_target_id(&mut self, target_id: &str) -> Result<(), BrowserError> {
        let target = self
            .targets
            .get(target_id)
            .cloned()
            .ok_or_else(|| BrowserError::InvalidTarget(target_id.to_string()))?;
        self.page
            .root_command("Target.activateTarget", json!({ "targetId": target_id }))
            .await?;
        self.page.activate(target_id.to_string(), target.session_id);
        Ok(())
    }

    pub(crate) async fn switch_target(
        &mut self,
        target_ref: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError> {
        self.refresh_browser_events(false).await?;
        let target_id = self
            .targets
            .iter()
            .find(|(_, target)| target.target_ref == target_ref)
            .map(|(id, _)| id.clone())
            .ok_or_else(|| BrowserError::InvalidTarget(target_ref.to_string()))?;
        self.activate_target_id(&target_id).await?;
        self.observations.clear();
        self.page_output("switch_target", json!({ "targetRef": target_ref }))
            .await
    }

    pub(crate) async fn observe(
        &mut self,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError> {
        self.refresh_browser_events(true).await?;
        let active_target = self.active_target()?;
        let url = self.current_url().await?;
        let title = self.current_title().await?;
        if let Some(target) = self
            .targets
            .get_mut(self.page.target_id.as_deref().unwrap_or_default())
        {
            target.url = url.clone();
            target.title = title.clone();
        }
        let capture = self.capture_documents().await?;
        let (text, text_truncated) = truncate_utf8(&capture.text, self.max_snapshot_bytes);
        let observation_id = BrowserObservationId::new();
        let mut stored_nodes = HashMap::new();
        let nodes = capture
            .nodes
            .into_iter()
            .map(|capture| {
                let node_ref = BrowserNodeRef::new();
                let locator = capture
                    .locator
                    .expect("captured browser nodes have locators");
                let node = BrowserNode {
                    node_ref,
                    role: capture.role.unwrap_or_else(|| capture.tag_name.clone()),
                    name: capture.name.unwrap_or_default(),
                    tag_name: capture.tag_name,
                    bounds: capture.bounds,
                    target_ref: Some(locator.target_ref.clone()),
                    frame_ref: capture.frame_ref,
                    href: capture.href,
                    form_action: capture.form_action,
                    form_method: capture.form_method,
                    input_type: capture.input_type,
                    editable: capture.editable,
                    requires_user_action: capture.requires_user_action,
                    user_action_reason: capture.user_action_reason,
                };
                stored_nodes.insert(
                    node_ref,
                    LocalBrowserNode {
                        node: node.clone(),
                        locator,
                    },
                );
                node
            })
            .collect::<Vec<_>>();
        self.prune_observations();
        self.observations.insert(
            observation_id,
            LocalBrowserObservation {
                captured_at: Instant::now(),
                url: url.clone(),
                target_ref: active_target.target_ref.clone(),
                nodes: stored_nodes,
            },
        );
        let screenshot = if options.include_screenshot {
            let (bytes, _) = self.screenshot_bytes().await?;
            Some(BrowserScreenshot {
                mime_type: "image/png".to_string(),
                bytes,
            })
        } else {
            None
        };
        let targets = self.browser_targets();
        let accessibility_tree = self.capture_accessibility_tree(&capture.frames).await?;
        Ok(BrowserObservation {
            observation_id,
            url,
            title,
            text,
            text_truncated,
            nodes,
            targets,
            frames: capture.frames,
            accessibility_tree,
            dialogs: std::mem::take(&mut self.dialogs),
            screenshot,
        })
    }

    pub(crate) async fn screenshot(&mut self) -> Result<BrowserOutput, BrowserError> {
        let (bytes, capture_backend) = self.screenshot_bytes().await?;
        Ok(BrowserOutput {
            url: Some(self.current_url().await?),
            contents: vec![BrowserContent::Image {
                mime_type: "image/png".to_string(),
                bytes,
            }],
            metadata: json!({ "action": "screenshot", "captureBackend": capture_backend }),
        })
    }

    async fn screenshot_bytes(&mut self) -> Result<(Vec<u8>, &'static str), BrowserError> {
        let primary = self.capture_screenshot(true).await?;
        if !png_looks_blank(&primary)? {
            return Ok((primary, "surface"));
        }
        let fallback = self.capture_screenshot(false).await?;
        if png_looks_blank(&fallback)? {
            return Err(BrowserError::Protocol(
                "Both screenshot backends returned an empty or blank image".to_string(),
            ));
        }
        Ok((fallback, "view"))
    }

    async fn capture_screenshot(&mut self, from_surface: bool) -> Result<Vec<u8>, BrowserError> {
        let result = self
            .page
            .command(
                "Page.captureScreenshot",
                json!({ "format": "png", "fromSurface": from_surface }),
            )
            .await?;
        let encoded = result.get("data").and_then(Value::as_str).ok_or_else(|| {
            BrowserError::Protocol("Page.captureScreenshot returned no image data".to_string())
        })?;
        let bytes = BASE64_STANDARD
            .decode(encoded)
            .map_err(|error| BrowserError::Protocol(format!("Invalid screenshot data: {error}")))?;
        if bytes.len() > self.max_screenshot_bytes {
            return Err(BrowserError::ScreenshotTooLarge {
                actual: bytes.len(),
                maximum: self.max_screenshot_bytes,
            });
        }
        Ok(bytes)
    }

    async fn perform_locator(
        &self,
        locator: &LocalNodeLocator,
        action: &BrowserAction,
    ) -> Result<(), BrowserError> {
        let path = serde_json::to_string(&locator.selector_path)?;
        let (operation, value, clear_first, delta_x, delta_y) = match action {
            BrowserAction::Click => ("click", None, false, 0.0, 0.0),
            BrowserAction::Type { text, clear_first } => {
                ("type", Some(text.as_str()), *clear_first, 0.0, 0.0)
            }
            BrowserAction::Select { value } => ("select", Some(value.as_str()), false, 0.0, 0.0),
            BrowserAction::Hover => ("hover", None, false, 0.0, 0.0),
            BrowserAction::Scroll { delta_x, delta_y } => {
                ("scroll", None, false, *delta_x, *delta_y)
            }
        };
        let operation = serde_json::to_string(operation)?;
        let value = serde_json::to_string(value.unwrap_or_default())?;
        let expression = format!(
            r#"(() => {{
  const path = {path};
  let root = document;
  let element = null;
  for (let index = 0; index < path.length; index += 1) {{
    element = root.querySelector(path[index]);
    if (!element) return {{ found: false }};
    if (index + 1 < path.length) {{
      root = element.shadowRoot;
      if (!root) return {{ found: false }};
    }}
  }}
  element.scrollIntoView({{ block: 'center', inline: 'center' }});
  const operation = {operation};
  const value = {value};
  if (operation === 'click') {{
    element.click();
  }} else if (operation === 'type') {{
    element.focus();
    if (element.isContentEditable) {{
      element.textContent = {clear_first} ? value : String(element.textContent || '') + value;
    }} else if ('value' in element) {{
      const next = {clear_first} ? value : String(element.value || '') + value;
      const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(element), 'value');
      if (descriptor && descriptor.set) descriptor.set.call(element, next); else element.value = next;
    }} else return {{ found: true, supported: false }};
    element.dispatchEvent(new InputEvent('input', {{ bubbles: true, composed: true, inputType: 'insertText', data: value }}));
    element.dispatchEvent(new Event('change', {{ bubbles: true, composed: true }}));
  }} else if (operation === 'select') {{
    if (!(element instanceof HTMLSelectElement)) return {{ found: true, supported: false }};
    const option = Array.from(element.options).find((candidate) => candidate.value === value || candidate.label === value);
    if (!option) return {{ found: true, optionFound: false }};
    element.value = option.value;
    element.dispatchEvent(new Event('input', {{ bubbles: true, composed: true }}));
    element.dispatchEvent(new Event('change', {{ bubbles: true, composed: true }}));
  }} else if (operation === 'hover') {{
    const rect = element.getBoundingClientRect();
    const init = {{ bubbles: true, composed: true, clientX: rect.x + rect.width / 2, clientY: rect.y + rect.height / 2 }};
    if (typeof PointerEvent === 'function') element.dispatchEvent(new PointerEvent('pointerover', init));
    element.dispatchEvent(new MouseEvent('mouseover', init));
    element.dispatchEvent(new MouseEvent('mouseenter', {{ ...init, bubbles: false }}));
    element.dispatchEvent(new MouseEvent('mousemove', init));
  }} else if (operation === 'scroll') {{
    const scroller = element.scrollHeight > element.clientHeight || element.scrollWidth > element.clientWidth ? element : window;
    if ({delta_x} !== 0 || {delta_y} !== 0) scroller.scrollBy({{ left: {delta_x}, top: {delta_y}, behavior: 'instant' }});
  }}
  return {{ found: true, supported: true, optionFound: true }};
}})()"#
        );
        let result = self
            .evaluate_value_for_session(&locator.session_id, Some(locator.context_id), &expression)
            .await?;
        if !result
            .get("found")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(BrowserError::StaleObservation {
                reason: "the observed element no longer exists".to_string(),
            });
        }
        if result.get("supported").and_then(Value::as_bool) == Some(false) {
            return Err(BrowserError::Protocol(format!(
                "the observed element does not support {}",
                action.name()
            )));
        }
        if result.get("optionFound").and_then(Value::as_bool) == Some(false) {
            return Err(BrowserError::Protocol(
                "the requested select option does not exist".to_string(),
            ));
        }
        Ok(())
    }

    pub(crate) async fn observation_node(
        &mut self,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError> {
        Ok(self.observed_node(observation_id, node_ref)?.node)
    }

    pub(crate) async fn perform(
        &mut self,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
        action: BrowserAction,
    ) -> Result<BrowserActionReceipt, BrowserError> {
        self.refresh_browser_events(false).await?;
        let (observed_url, observed_target_ref, stored) =
            self.observed_node_with_url(observation_id, node_ref)?;
        if self.active_target()?.target_ref != observed_target_ref {
            return Err(BrowserError::StaleObservation {
                reason: "the active browser target changed after the observation".to_string(),
            });
        }
        let current_url = self.current_url().await?;
        if current_url != observed_url {
            return Err(BrowserError::StaleObservation {
                reason: "the page URL changed after the observation".to_string(),
            });
        }
        let current_capture = self
            .capture_documents()
            .await?
            .nodes
            .into_iter()
            .find(|node| {
                node.locator.as_ref().is_some_and(|locator| {
                    locator.target_ref == stored.locator.target_ref
                        && locator.frame_id == stored.locator.frame_id
                        && locator.selector_path == stored.locator.selector_path
                        && locator.node_key == stored.locator.node_key
                })
            })
            .ok_or_else(|| BrowserError::StaleObservation {
                reason: "the observed element no longer exists".to_string(),
            })?;
        let current_locator = current_capture
            .locator
            .clone()
            .expect("captured browser nodes have locators");
        let current = BrowserNode {
            node_ref,
            role: current_capture
                .role
                .unwrap_or_else(|| current_capture.tag_name.clone()),
            name: current_capture.name.unwrap_or_default(),
            tag_name: current_capture.tag_name,
            bounds: current_capture.bounds,
            target_ref: Some(current_locator.target_ref.clone()),
            frame_ref: current_capture.frame_ref,
            href: current_capture.href,
            form_action: current_capture.form_action,
            form_method: current_capture.form_method,
            input_type: current_capture.input_type,
            editable: current_capture.editable,
            requires_user_action: current_capture.requires_user_action,
            user_action_reason: current_capture.user_action_reason,
        };
        if !stored.node.matches_current(&current) {
            return Err(BrowserError::StaleObservation {
                reason: "the observed element changed or moved".to_string(),
            });
        }

        let before = self.page_state().await?;
        if matches!(action, BrowserAction::Type { .. }) && !current.editable {
            return Err(BrowserError::StaleObservation {
                reason: "the observed element is no longer editable".to_string(),
            });
        }
        self.perform_locator(&current_locator, &action).await?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.refresh_browser_events(true).await?;
        let after = self.page_state().await?;
        let verification = BrowserActionVerification {
            url_changed: before.url != after.url,
            title_changed: before.title != after.title,
            text_changed: before.text != after.text,
            page_changed: before.url != after.url
                || before.title != after.title
                || before.text != after.text,
        };
        Ok(BrowserActionReceipt {
            observation_id,
            node_ref,
            action: action.name().to_string(),
            target: current,
            url: after.url,
            title: after.title,
            verification,
        })
    }

    async fn page_state(&mut self) -> Result<BrowserPageState, BrowserError> {
        serde_json::from_value(
            self.evaluate_value(
                "({ url: document.location.href, title: document.title, text: (document.body ? document.body.innerText : '').slice(0, 262144) })",
            )
            .await?,
        )
        .map_err(BrowserError::Json)
    }

    fn prune_observations(&mut self) {
        self.observations
            .retain(|_, observation| observation.captured_at.elapsed() <= OBSERVATION_TTL);
        if self.observations.len() <= MAX_OBSERVATIONS_PER_SESSION {
            return;
        }
        let mut oldest = self
            .observations
            .iter()
            .map(|(id, observation)| (*id, observation.captured_at))
            .collect::<Vec<_>>();
        oldest.sort_by_key(|(_, captured_at)| *captured_at);
        for (id, _) in oldest.into_iter().take(
            self.observations
                .len()
                .saturating_sub(MAX_OBSERVATIONS_PER_SESSION),
        ) {
            self.observations.remove(&id);
        }
    }

    fn observed_node(
        &mut self,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<LocalBrowserNode, BrowserError> {
        Ok(self.observed_node_with_url(observation_id, node_ref)?.2)
    }

    fn observed_node_with_url(
        &mut self,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<(String, BrowserTargetRef, LocalBrowserNode), BrowserError> {
        self.prune_observations();
        let observation = self.observations.get(&observation_id).ok_or_else(|| {
            BrowserError::StaleObservation {
                reason: "the observation is missing or expired".to_string(),
            }
        })?;
        let node = observation.nodes.get(&node_ref).cloned().ok_or_else(|| {
            BrowserError::StaleObservation {
                reason: "the node does not belong to this observation".to_string(),
            }
        })?;
        Ok((
            observation.url.clone(),
            observation.target_ref.clone(),
            node,
        ))
    }

    fn browser_targets(&self) -> Vec<BrowserTarget> {
        let active = self.page.target_id.as_deref();
        let mut targets = self
            .targets
            .iter()
            .map(|(target_id, target)| BrowserTarget {
                target_ref: target.target_ref.clone(),
                url: target.url.clone(),
                title: target.title.clone(),
                active: active == Some(target_id.as_str()),
                opener: target
                    .opener_target_id
                    .as_ref()
                    .and_then(|opener| self.targets.get(opener))
                    .map(|opener| opener.target_ref.clone()),
            })
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| left.target_ref.as_str().cmp(right.target_ref.as_str()));
        targets
    }

    async fn capture_documents(&mut self) -> Result<LocalDocumentCapture, BrowserError> {
        let target_id = self.page.target_id.clone().ok_or_else(|| {
            BrowserError::Protocol("No active browser target is attached".to_string())
        })?;
        let target = self.active_target()?;
        let tree = self
            .page
            .command_for_session(
                "Page.getFrameTree",
                json!({}),
                Some(target.session_id.clone()),
            )
            .await?;
        let mut raw_frames = Vec::new();
        collect_cdp_frames(
            tree.get("frameTree").unwrap_or(&Value::Null),
            None,
            &mut raw_frames,
        );
        for (frame_id, _, _, _) in &raw_frames {
            self.frame_refs
                .entry((target_id.clone(), frame_id.clone()))
                .or_insert_with(BrowserFrameRef::new);
        }
        let frames = raw_frames
            .iter()
            .map(|(frame_id, parent_id, url, name)| BrowserFrame {
                frame_ref: self.frame_refs[&(target_id.clone(), frame_id.clone())].clone(),
                target_ref: target.target_ref.clone(),
                parent_frame_ref: parent_id.as_ref().and_then(|parent| {
                    self.frame_refs
                        .get(&(target_id.clone(), parent.clone()))
                        .cloned()
                }),
                url: url.clone(),
                name: name.clone(),
            })
            .collect::<Vec<_>>();
        let root_frame_id = raw_frames.first().map(|frame| frame.0.clone());
        let mut text = String::new();
        let mut nodes = Vec::new();
        for (frame_id, _, _, _) in raw_frames {
            let world = match self
                .page
                .command_for_session(
                    "Page.createIsolatedWorld",
                    json!({
                        "frameId": frame_id,
                        "worldName": "opentopia-browser-agent",
                        "grantUniversalAccess": false,
                    }),
                    Some(target.session_id.clone()),
                )
                .await
            {
                Ok(world) => world,
                Err(_) if root_frame_id.as_deref() != Some(frame_id.as_str()) => continue,
                Err(error) => return Err(error),
            };
            let context_id = world
                .get("executionContextId")
                .and_then(Value::as_i64)
                .ok_or_else(|| {
                    BrowserError::Protocol(
                        "Page.createIsolatedWorld returned no execution context".to_string(),
                    )
                })?;
            let value = match self
                .evaluate_value_for_session(
                    &target.session_id,
                    Some(context_id),
                    INTERACTIVE_SNAPSHOT_SCRIPT,
                )
                .await
            {
                Ok(value) => value,
                Err(_) if root_frame_id.as_deref() != Some(frame_id.as_str()) => continue,
                Err(error) => return Err(error),
            };
            if let Some(frame_text) = value.get("text").and_then(Value::as_str) {
                if !frame_text.trim().is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(frame_text);
                }
            }
            let raw_nodes = value.get("nodes").cloned().unwrap_or_else(|| json!([]));
            let mut frame_nodes: Vec<CapturedBrowserNode> = serde_json::from_value(raw_nodes)
                .map_err(|error| {
                    BrowserError::Protocol(format!(
                        "browser observation nodes are invalid: {error}"
                    ))
                })?;
            let frame_ref = self.frame_refs[&(target_id.clone(), frame_id.clone())].clone();
            for node in &mut frame_nodes {
                node.frame_ref = Some(frame_ref.clone());
                node.locator = Some(LocalNodeLocator {
                    target_ref: target.target_ref.clone(),
                    session_id: target.session_id.clone(),
                    frame_id: frame_id.clone(),
                    context_id,
                    selector_path: node.selector_path.clone(),
                    node_key: node.node_key.clone(),
                });
            }
            nodes.extend(
                frame_nodes
                    .into_iter()
                    .take(200_usize.saturating_sub(nodes.len())),
            );
            if nodes.len() >= 200 {
                break;
            }
        }
        Ok(LocalDocumentCapture {
            text,
            nodes,
            frames,
        })
    }

    async fn capture_accessibility_tree(
        &mut self,
        frames: &[BrowserFrame],
    ) -> Result<Vec<BrowserAccessibilityNode>, BrowserError> {
        let target = self.active_target()?;
        let target_id = self.page.target_id.clone().unwrap_or_default();
        let mut output = Vec::new();
        for frame in frames {
            let frame_id = self
                .frame_refs
                .iter()
                .find(|((owner, _), reference)| {
                    owner == &target_id && *reference == &frame.frame_ref
                })
                .map(|((_, frame_id), _)| frame_id.clone());
            let Some(frame_id) = frame_id else {
                continue;
            };
            let result = match self
                .page
                .command_for_session(
                    "Accessibility.getFullAXTree",
                    json!({ "depth": 32, "frameId": frame_id }),
                    Some(target.session_id.clone()),
                )
                .await
            {
                Ok(result) => result,
                Err(_) if frame.parent_frame_ref.is_some() => continue,
                Err(error) => return Err(error),
            };
            for node in result
                .get("nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(1_000_usize.saturating_sub(output.len()))
            {
                let Some(node_id) = node.get("nodeId").and_then(Value::as_str) else {
                    continue;
                };
                output.push(BrowserAccessibilityNode {
                    ax_node_id: format!("{}:{node_id}", frame.frame_ref),
                    parent_ax_node_id: node
                        .get("parentId")
                        .and_then(Value::as_str)
                        .map(|parent| format!("{}:{parent}", frame.frame_ref)),
                    role: cdp_ax_string(node.get("role")),
                    name: cdp_ax_string(node.get("name")),
                    value: cdp_ax_optional_string(node.get("value")),
                    description: cdp_ax_optional_string(node.get("description")),
                    ignored: node
                        .get("ignored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    target_ref: target.target_ref.clone(),
                    frame_ref: Some(frame.frame_ref.clone()),
                    node_ref: None,
                });
            }
            if output.len() >= 1_000 {
                break;
            }
        }
        Ok(output)
    }

    pub(crate) async fn wait(
        &mut self,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let timeout = request.timeout.unwrap_or(self.command_timeout);
        let poll_interval = if request.poll_interval.is_zero() {
            DEFAULT_WAIT_POLL_INTERVAL
        } else {
            request.poll_interval
        };
        let started = tokio::time::Instant::now();
        loop {
            let matched = match &request.condition {
                BrowserWaitCondition::DocumentComplete => self
                    .evaluate_value("document.readyState !== 'loading'")
                    .await?
                    .as_bool()
                    .unwrap_or(false),
                BrowserWaitCondition::Selector(selector) => {
                    let selector = serde_json::to_string(selector.as_str())?;
                    self.evaluate_value(&format!("Boolean(document.querySelector({selector}))"))
                        .await?
                        .as_bool()
                        .unwrap_or(false)
                }
                BrowserWaitCondition::Text(text) => {
                    let text = serde_json::to_string(text)?;
                    self.evaluate_value(&format!(
                        "Boolean(document.body && document.body.innerText.includes({text}))"
                    ))
                    .await?
                    .as_bool()
                    .unwrap_or(false)
                }
            };
            if matched {
                return self
                    .page_output(
                        "wait",
                        json!({ "condition": wait_condition_name(&request.condition) }),
                    )
                    .await;
            }
            if started.elapsed() >= timeout {
                return Err(BrowserError::Timeout(
                    wait_condition_name(&request.condition).to_string(),
                ));
            }
            tokio::time::sleep(poll_interval.min(Duration::from_millis(500))).await;
        }
    }

    pub(crate) async fn download(
        &mut self,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        if !self.supports_downloads {
            return Err(BrowserError::Protocol(
                "Downloads are not supported for an attached personal Chrome tab".to_string(),
            ));
        }
        let before = list_downloads(&self.download_dir).await?;
        self.page.discard_events();
        self.page
            .command("Page.navigate", json!({ "url": request.url }))
            .await?;
        let download = wait_for_download(
            &mut self.page,
            &self.download_dir,
            &before,
            request.expected_filename.as_deref(),
            request.timeout.unwrap_or(self.command_timeout),
            self.max_download_bytes,
        )
        .await?;
        Ok(BrowserOutput {
            url: Some(request.url),
            contents: vec![BrowserContent::File {
                path: download.path.clone(),
                mime_type: download.content_type.clone(),
                bytes: download.bytes,
            }],
            metadata: json!({ "action": "download", "filename": download.filename }),
        })
    }

    async fn page_output(
        &mut self,
        action: &str,
        details: Value,
    ) -> Result<BrowserOutput, BrowserError> {
        let navigation = BrowserNavigation {
            url: self.current_url().await?,
            title: self.current_title().await?,
        };
        Ok(BrowserOutput {
            url: Some(navigation.url.clone()),
            contents: vec![BrowserContent::Json {
                value: serde_json::to_value(navigation)?,
            }],
            metadata: json!({ "action": action, "details": details }),
        })
    }

    async fn current_url(&mut self) -> Result<String, BrowserError> {
        Ok(self
            .evaluate_value("document.location.href")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    async fn current_title(&mut self) -> Result<String, BrowserError> {
        Ok(self
            .evaluate_value("document.title")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    async fn evaluate_value(&mut self, expression: &str) -> Result<Value, BrowserError> {
        let session_id = self.page.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("No active CDP page session is attached".to_string())
        })?;
        self.evaluate_value_for_session(&session_id, None, expression)
            .await
    }

    async fn evaluate_value_for_session(
        &self,
        session_id: &str,
        context_id: Option<i64>,
        expression: &str,
    ) -> Result<Value, BrowserError> {
        let mut parameters = json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
            "userGesture": true,
        });
        if let Some(context_id) = context_id {
            parameters["contextId"] = json!(context_id);
        }
        let result = self
            .page
            .command_for_session("Runtime.evaluate", parameters, Some(session_id.to_string()))
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(BrowserError::Cdp {
                method: "Runtime.evaluate".to_string(),
                message: exception
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("JavaScript evaluation failed")
                    .to_string(),
            });
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub(crate) async fn shutdown(&mut self) -> Result<(), BrowserError> {
        if !self.owns_targets {
            let _ = self.page.root_command("OpenTopia.detach", json!({})).await;
            return Ok(());
        }
        let target_ids = self.targets.keys().cloned().collect::<Vec<_>>();
        for target_id in target_ids {
            let _ = self
                .page
                .root_command("Target.closeTarget", json!({ "targetId": target_id }))
                .await;
        }
        Ok(())
    }
}

fn collect_cdp_frames(
    tree: &Value,
    parent: Option<String>,
    output: &mut Vec<(String, Option<String>, String, String)>,
) {
    let frame = tree.get("frame").unwrap_or(&Value::Null);
    let Some(frame_id) = frame.get("id").and_then(Value::as_str) else {
        return;
    };
    output.push((
        frame_id.to_string(),
        parent,
        frame
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        frame
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    ));
    for child in tree
        .get("childFrames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        collect_cdp_frames(child, Some(frame_id.to_string()), output);
    }
}

fn cdp_ax_string(value: Option<&Value>) -> String {
    value
        .and_then(|value| value.get("value"))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Bool(value) => Some(value.to_string()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn cdp_ax_optional_string(value: Option<&Value>) -> Option<String> {
    let value = cdp_ax_string(value);
    (!value.is_empty()).then_some(value)
}

fn urls_equivalent(left: &str, right: &str) -> bool {
    match (reqwest::Url::parse(left), reqwest::Url::parse(right)) {
        (Ok(mut left), Ok(mut right)) => {
            left.set_fragment(None);
            right.set_fragment(None);
            left == right
        }
        _ => left == right,
    }
}

fn deserialize_browser_bytes<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    struct BrowserBytesVisitor;

    impl<'de> Visitor<'de> for BrowserBytesVisitor {
        type Value = Vec<u8>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a base64 string or an array of byte values")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            BASE64_STANDARD.decode(value).map_err(E::custom)
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            self.visit_str(&value)
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut bytes = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
            while let Some(byte) = sequence.next_element::<u8>()? {
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }

    deserializer.deserialize_any(BrowserBytesVisitor)
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> (String, bool) {
    if value.len() <= maximum_bytes {
        return (value.to_string(), false);
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

fn wait_condition_name(condition: &BrowserWaitCondition) -> &'static str {
    match condition {
        BrowserWaitCondition::DocumentComplete => "document_complete",
        BrowserWaitCondition::Selector(_) => "selector",
        BrowserWaitCondition::Text(_) => "text",
    }
}

const INTERACTIVE_SNAPSHOT_SCRIPT: &str = r#"
(() => {
  const max = 200;
  const identities = globalThis.__opentopiaBrowserNodeIdentities ||
    (globalThis.__opentopiaBrowserNodeIdentities = { nodes: new WeakMap(), next: 0 });
  const nodeKey = (element) => {
    let key = identities.nodes.get(element);
    if (!key) {
      key = String(++identities.next);
      identities.nodes.set(element, key);
    }
    return key;
  };
  const text = (element) => (element.innerText || element.value || element.getAttribute('aria-label') || element.getAttribute('placeholder') || '')
    .replace(/\s+/g, ' ').trim().slice(0, 240);
  const role = (element) => element.getAttribute('role') || {
    a: 'link', button: 'button', input: element.type === 'checkbox' ? 'checkbox' : element.type === 'radio' ? 'radio' : 'textbox',
    textarea: 'textbox', select: 'combobox'
  }[element.tagName.toLowerCase()] || element.tagName.toLowerCase();
  const cssPath = (element, root) => {
    if (element.id && window.CSS && CSS.escape) return `#${CSS.escape(element.id)}`;
    const parts = [];
    for (let node = element; node && node.nodeType === Node.ELEMENT_NODE; node = node.parentElement) {
      let part = node.tagName.toLowerCase();
      const siblings = Array.from(node.parentElement?.children || []).filter((child) => child.tagName === node.tagName);
      if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
      parts.unshift(part);
      if (node.parentNode === root || node === document.body) break;
    }
    return parts.join(' > ');
  };
  const handoffReason = (element, label, inputType, formMethod) => {
    const normalized = `${label} ${element.getAttribute('aria-label') || ''} ${element.getAttribute('title') || ''}`.toLowerCase();
    if (inputType === 'file') return 'Please choose and upload the file yourself in the visible browser, then tell me to continue.';
    if (inputType === 'password' || /sign[ -]?in|log[ -]?in|password|passkey|verification|verify|captcha|one[ -]?time code|security code/.test(normalized)) {
      return 'Please complete the sign-in or verification step yourself in the visible browser, then tell me to continue.';
    }
    if (/pay|payment|checkout|purchase|buy now|place order|subscribe/.test(normalized)) {
      return 'Please review and complete the payment or purchase yourself in the visible browser, then tell me to continue.';
    }
    if (/send|publish|post|share|upload|delete|remove|submit|save changes|confirm/.test(normalized) && formMethod !== 'get') {
      return 'Please review and complete this external action yourself in the visible browser, then tell me to continue.';
    }
    return null;
  };
  const nodes = [];
  const selector = 'a[href], button, input, textarea, select, [role="button"], [role="link"], [contenteditable="true"], [tabindex]';
  const walk = (root, shadowPath) => {
    for (const element of root.querySelectorAll(selector)) {
      if (nodes.length >= max) break;
      if (element.disabled || !element.getClientRects().length) continue;
      const inputType = (element.getAttribute('type') || '').toLowerCase() || null;
      const formMethod = (element.getAttribute('formmethod') || element.form?.getAttribute('method') || 'get').toLowerCase();
      const name = text(element);
      const userActionReason = handoffReason(element, name, inputType, formMethod);
      nodes.push({
        selectorPath: [...shadowPath, cssPath(element, root)], nodeKey: nodeKey(element), tagName: element.tagName.toLowerCase(), role: role(element), name,
        href: element.href || null, formAction: element.formAction || (element.form && element.form.action) || null,
        formMethod, inputType,
        editable: Boolean(element.isContentEditable || ['input', 'textarea', 'select'].includes(element.tagName.toLowerCase()) && !element.readOnly),
        requiresUserAction: Boolean(userActionReason), userActionReason,
        bounds: (() => { const rect = element.getBoundingClientRect(); return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }; })()
      });
    }
    for (const host of root.querySelectorAll('*')) {
      if (nodes.length >= max) break;
      if (host.shadowRoot) walk(host.shadowRoot, [...shadowPath, cssPath(host, root)]);
    }
  };
  walk(document, []);
  return { text: document.body ? document.body.innerText : '', nodes };
})()
"#;

#[cfg(test)]
mod tests;
