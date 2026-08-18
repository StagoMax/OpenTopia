//! Stable browser runtime contracts shared by managed and attached-browser backends.

use super::io_support::normalize_network_host;
use super::{
    deserialize_browser_bytes, DEFAULT_COMMAND_TIMEOUT, DEFAULT_MAX_DOWNLOAD_BYTES,
    DEFAULT_MAX_SCREENSHOT_BYTES, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_STARTUP_TIMEOUT,
    DEFAULT_WAIT_POLL_INTERVAL, MAX_NETWORK_HOSTS, MAX_NODE_POSITION_DRIFT,
};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

/// An opaque ID that should normally be derived from a thread ID. A session owns one initial tab
/// plus any popups it creates; all sessions share the browser profile, cookie jar, cache, and
/// downloads without sharing target ownership or observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserSessionId(Uuid);

impl BrowserSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_thread(thread_id: Uuid) -> Self {
        Self(thread_id)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for BrowserSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BrowserSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

pub(super) const DEFAULT_BROWSER_PROFILE_ID: &str = "default";
const MAX_BROWSER_PROFILE_ID_LEN: usize = 64;

/// A stable, filesystem- and partition-safe browser profile identifier.
///
/// Profile IDs are product-level identities, not Chromium directory names. Each backend derives
/// its own storage location from this value and must never accept a caller-provided path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct BrowserProfileId(String);

impl BrowserProfileId {
    pub fn new(value: impl Into<String>) -> Result<Self, BrowserError> {
        let value = value.into();
        let mut chars = value.chars();
        let valid = value.len() <= MAX_BROWSER_PROFILE_ID_LEN
            && chars
                .next()
                .is_some_and(|character| character.is_ascii_alphanumeric())
            && chars.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
            });
        if !valid {
            return Err(BrowserError::InvalidProfileId(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for BrowserProfileId {
    fn default() -> Self {
        Self(DEFAULT_BROWSER_PROFILE_ID.to_string())
    }
}

impl fmt::Display for BrowserProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for BrowserProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserProfilePersistence {
    Persistent,
    Ephemeral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBackendKind {
    LocalChrome,
    Electron,
    ChromeExtension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserSurfaceKind {
    Headless,
    ExternalWindow,
    Embedded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserActionCapability {
    Navigate,
    Observe,
    SwitchTarget,
    Click,
    Type,
    Select,
    Hover,
    Scroll,
    Screenshot,
    Wait,
    Download,
}

/// Capabilities are explicit because an attached personal Chrome tab cannot honestly promise the
/// same isolation properties as a managed Electron or local Chrome profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRuntimeCapabilities {
    pub protocol_version: u32,
    pub backend: BrowserBackendKind,
    pub surface: BrowserSurfaceKind,
    pub profile_persistence: Vec<BrowserProfilePersistence>,
    pub actions: Vec<BrowserActionCapability>,
    pub hard_network_isolation: bool,
    pub supports_user_handoff: bool,
    pub supports_external_profile: bool,
}

impl BrowserRuntimeCapabilities {
    pub(crate) fn managed(
        backend: BrowserBackendKind,
        surface: BrowserSurfaceKind,
        supports_user_handoff: bool,
    ) -> Self {
        Self {
            protocol_version: 1,
            backend,
            surface,
            profile_persistence: vec![
                BrowserProfilePersistence::Persistent,
                BrowserProfilePersistence::Ephemeral,
            ],
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
                BrowserActionCapability::Download,
            ],
            hard_network_isolation: true,
            supports_user_handoff,
            supports_external_profile: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSessionSpec {
    pub session_id: BrowserSessionId,
    #[serde(default)]
    pub profile_id: BrowserProfileId,
    pub profile_persistence: BrowserProfilePersistence,
}

impl BrowserSessionSpec {
    pub fn persistent(session_id: BrowserSessionId, profile_id: BrowserProfileId) -> Self {
        Self {
            session_id,
            profile_id,
            profile_persistence: BrowserProfilePersistence::Persistent,
        }
    }

    pub fn ephemeral(session_id: BrowserSessionId) -> Self {
        Self {
            session_id,
            profile_id: BrowserProfileId::new(format!("session-{session_id}"))
                .expect("UUID-derived browser profile IDs are valid"),
            profile_persistence: BrowserProfilePersistence::Ephemeral,
        }
    }
}

impl From<BrowserSessionId> for BrowserSessionSpec {
    fn from(session_id: BrowserSessionId) -> Self {
        Self::persistent(session_id, BrowserProfileId::default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSessionInfo {
    pub session_id: BrowserSessionId,
    pub profile_id: BrowserProfileId,
    pub profile_persistence: BrowserProfilePersistence,
    pub backend: BrowserBackendKind,
}

/// A capability grant applied at the browser driver boundary. Hosts are exact, normalized DNS
/// names or IP addresses. Grants accumulate only within one browser session and are discarded
/// when that session closes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNetworkGrant {
    pub allowed_hosts: Vec<String>,
}

impl BrowserNetworkGrant {
    pub fn new<I, S>(hosts: I) -> Result<Self, BrowserError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut allowed_hosts = HashSet::new();
        for host in hosts {
            allowed_hosts.insert(normalize_network_host(host.as_ref())?);
        }
        if allowed_hosts.len() > MAX_NETWORK_HOSTS {
            return Err(BrowserError::InvalidNetworkGrant(format!(
                "at most {MAX_NETWORK_HOSTS} hosts may be authorized per browser session"
            )));
        }
        let mut allowed_hosts = allowed_hosts.into_iter().collect::<Vec<_>>();
        allowed_hosts.sort();
        Ok(Self { allowed_hosts })
    }

    pub(super) fn merge(&mut self, other: Self) -> Result<(), BrowserError> {
        let mut hosts = self.allowed_hosts.iter().cloned().collect::<HashSet<_>>();
        for host in other.allowed_hosts {
            hosts.insert(normalize_network_host(&host)?);
        }
        if hosts.len() > MAX_NETWORK_HOSTS {
            return Err(BrowserError::InvalidNetworkGrant(format!(
                "at most {MAX_NETWORK_HOSTS} hosts may be authorized per browser session"
            )));
        }
        self.allowed_hosts = hosts.into_iter().collect();
        self.allowed_hosts.sort();
        Ok(())
    }
}

/// Opaque token for one point-in-time browser observation. It is valid only for a short period
/// and only within the browser session that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserObservationId(Uuid);

impl BrowserObservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for BrowserObservationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque reference to an interactive node in one `BrowserObservation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserNodeRef(Uuid);

impl BrowserNodeRef {
    pub(super) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for BrowserNodeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque reference to a page target owned by one browser session. Targets are tabs or popups;
/// the backing Chromium target identifiers never need to be exposed to the caller.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserTargetRef(String);

impl BrowserTargetRef {
    pub(super) fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BrowserTargetRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Opaque reference to a document frame within one target.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserFrameRef(String);

impl BrowserFrameRef {
    pub(super) fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl fmt::Display for BrowserFrameRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTarget {
    pub target_ref: BrowserTargetRef,
    pub url: String,
    pub title: String,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<BrowserTargetRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserFrame {
    pub frame_ref: BrowserFrameRef,
    pub target_ref: BrowserTargetRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_frame_ref: Option<BrowserFrameRef>,
    pub url: String,
    pub name: String,
}

/// A bounded accessibility-tree node. `node_ref` links an AX node to an actionable observation
/// node when Chromium exposes a stable backend DOM node identity for both representations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAccessibilityNode {
    pub ax_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_ax_node_id: Option<String>,
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub ignored: bool,
    pub target_ref: BrowserTargetRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_ref: Option<BrowserFrameRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_ref: Option<BrowserNodeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDialog {
    pub dialog_type: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_prompt: Option<String>,
    /// Dialogs are dismissed by the driver so an unattended task cannot deadlock Chromium.
    #[serde(default)]
    pub handled: bool,
    pub target_ref: BrowserTargetRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BrowserRect {
    pub(super) fn materially_differs_from(&self, other: &Self) -> bool {
        (self.x - other.x).abs() > MAX_NODE_POSITION_DRIFT
            || (self.y - other.y).abs() > MAX_NODE_POSITION_DRIFT
            || (self.width - other.width).abs() > MAX_NODE_POSITION_DRIFT
            || (self.height - other.height).abs() > MAX_NODE_POSITION_DRIFT
    }
}

impl BrowserNode {
    pub(super) fn matches_current(&self, current: &Self) -> bool {
        self.role == current.role
            && self.name == current.name
            && self.tag_name == current.tag_name
            && self.target_ref == current.target_ref
            && self.frame_ref == current.frame_ref
            && self.href == current.href
            && self.form_action == current.form_action
            && self.form_method == current.form_method
            && self.input_type == current.input_type
            && self.editable == current.editable
            && self.requires_user_action == current.requires_user_action
            && self.user_action_reason == current.user_action_reason
            && !self.bounds.materially_differs_from(&current.bounds)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNode {
    pub node_ref: BrowserNodeRef,
    pub role: String,
    pub name: String,
    pub tag_name: String,
    pub bounds: BrowserRect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<BrowserTargetRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_ref: Option<BrowserFrameRef>,
    pub href: Option<String>,
    pub form_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_type: Option<String>,
    pub editable: bool,
    #[serde(default)]
    pub requires_user_action: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_action_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserScreenshot {
    pub mime_type: String,
    #[serde(deserialize_with = "deserialize_browser_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserObservation {
    pub observation_id: BrowserObservationId,
    pub url: String,
    pub title: String,
    pub text: String,
    pub text_truncated: bool,
    pub nodes: Vec<BrowserNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<BrowserTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<BrowserFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accessibility_tree: Vec<BrowserAccessibilityNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialogs: Vec<BrowserDialog>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<BrowserScreenshot>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowserObserveOptions {
    pub include_screenshot: bool,
}

#[derive(Debug, Clone)]
pub enum BrowserAction {
    Click,
    Type { text: String, clear_first: bool },
    Select { value: String },
    Hover,
    Scroll { delta_x: f64, delta_y: f64 },
}

impl BrowserAction {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Type { .. } => "type",
            Self::Select { .. } => "select",
            Self::Hover => "hover",
            Self::Scroll { .. } => "scroll",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionReceipt {
    pub observation_id: BrowserObservationId,
    pub node_ref: BrowserNodeRef,
    pub action: String,
    pub target: BrowserNode,
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub verification: BrowserActionVerification,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionVerification {
    pub page_changed: bool,
    pub url_changed: bool,
    pub title_changed: bool,
    pub text_changed: bool,
}

/// Configuration for a local Chrome or Edge process.
#[derive(Debug, Clone)]
pub struct BrowserRuntimeConfig {
    /// Use a specific Chrome/Edge executable. When omitted, Chrome and Edge are discovered from
    /// standard platform locations and `PATH` when a session is first used.
    pub executable: Option<PathBuf>,
    /// Browser state is stored below this directory in one shared profile directory.
    pub data_root: PathBuf,
    pub headless: bool,
    pub startup_timeout: Duration,
    pub command_timeout: Duration,
    pub max_snapshot_bytes: usize,
    pub max_screenshot_bytes: usize,
    pub max_download_bytes: u64,
    /// Navigation and direct-download URLs are restricted to these schemes. Domain approval is
    /// deliberately left to the caller's policy layer.
    pub allowed_schemes: Vec<String>,
    /// Preserve the shared browser profile and downloads after closing a tab. Defaults to true so
    /// browser login state behaves like a normal browser session.
    pub retain_session_data: bool,
}

impl Default for BrowserRuntimeConfig {
    fn default() -> Self {
        Self {
            executable: None,
            data_root: std::env::temp_dir().join("opentopia-browser"),
            headless: true,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            max_screenshot_bytes: DEFAULT_MAX_SCREENSHOT_BYTES,
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            retain_session_data: true,
        }
    }
}

/// Content that can later be passed straight into a multimodal tool-result/message contract.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserContent {
    Text {
        text: String,
        truncated: bool,
    },
    Json {
        value: Value,
    },
    Image {
        mime_type: String,
        #[serde(deserialize_with = "deserialize_browser_bytes")]
        bytes: Vec<u8>,
    },
    File {
        path: PathBuf,
        mime_type: Option<String>,
        bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOutput {
    pub url: Option<String>,
    pub contents: Vec<BrowserContent>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BrowserSelector(String);

impl BrowserSelector {
    pub fn new(selector: impl Into<String>) -> Result<Self, BrowserError> {
        let selector = selector.into();
        if selector.trim().is_empty() {
            return Err(BrowserError::InvalidSelector(
                "A CSS selector cannot be empty.".to_string(),
            ));
        }
        Ok(Self(selector))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct BrowserNavigateRequest {
    pub url: String,
    pub wait: Option<BrowserWaitRequest>,
}

impl BrowserNavigateRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            wait: Some(BrowserWaitRequest::document_complete()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrowserTypeRequest {
    pub selector: BrowserSelector,
    pub text: String,
    pub clear_first: bool,
}

#[derive(Debug, Clone)]
pub enum BrowserWaitCondition {
    DocumentComplete,
    Selector(BrowserSelector),
    Text(String),
}

#[derive(Debug, Clone)]
pub struct BrowserWaitRequest {
    pub condition: BrowserWaitCondition,
    pub timeout: Option<Duration>,
    pub poll_interval: Duration,
}

impl BrowserWaitRequest {
    pub fn document_complete() -> Self {
        Self {
            condition: BrowserWaitCondition::DocumentComplete,
            timeout: None,
            poll_interval: DEFAULT_WAIT_POLL_INTERVAL,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BrowserDownloadRequest {
    pub url: String,
    pub expected_filename: Option<String>,
    pub timeout: Option<Duration>,
}

impl BrowserDownloadRequest {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            expected_filename: None,
            timeout: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNavigation {
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDownload {
    pub path: PathBuf,
    pub filename: String,
    pub bytes: u64,
    pub content_type: Option<String>,
}

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("No supported local Chrome or Edge executable was found. Configure browser.executable or set OPENTOPIA_BROWSER_EXECUTABLE.")]
    ExecutableNotFound,
    #[error("Configured browser executable does not exist: {0}")]
    ExecutableMissing(PathBuf),
    #[error("Browser session was not found: {0}")]
    SessionNotFound(BrowserSessionId),
    #[error("Invalid browser profile ID: {0}")]
    InvalidProfileId(String),
    #[error("Browser session {session} is already bound to a different profile")]
    SessionProfileConflict { session: BrowserSessionId },
    #[error("Invalid browser URL: {0}")]
    InvalidUrl(String),
    #[error("Invalid browser network grant: {0}")]
    InvalidNetworkGrant(String),
    #[error("URL scheme is not allowed by this browser runtime: {0}")]
    DisallowedScheme(String),
    #[error("Invalid CSS selector: {0}")]
    InvalidSelector(String),
    #[error("stale_observation: {reason}")]
    StaleObservation { reason: String },
    #[error("Browser startup timed out after {0:?}")]
    StartupTimeout(Duration),
    #[error("Browser operation timed out while waiting for {0}")]
    Timeout(String),
    #[error("Browser protocol error: {0}")]
    Protocol(String),
    #[error("Browser disconnected: {0}")]
    Disconnected(String),
    #[error("Browser command {method} failed: {message}")]
    Cdp { method: String, message: String },
    #[error("Screenshot is {actual} bytes, exceeding the configured {maximum}-byte limit")]
    ScreenshotTooLarge { actual: usize, maximum: usize },
    #[error("Download did not complete before the timeout")]
    DownloadTimeout,
    #[error("Download exceeded the configured {maximum}-byte limit")]
    DownloadTooLarge { maximum: u64 },
    #[error("Browser network request was blocked because host `{host}` is not authorized")]
    NetworkBlocked { host: String },
    #[error("Browser target is missing or no longer owned by this session: {0}")]
    InvalidTarget(String),
    #[error("Desktop browser broker configuration is invalid: {0}")]
    BrokerConfiguration(String),
    #[error("Desktop browser broker is unavailable")]
    BrokerUnavailable,
    #[error("Desktop browser broker rejected the request with HTTP {status}: {message}")]
    BrokerRejected { status: u16, message: String },
    #[error(
        "Desktop browser broker response is {actual} bytes, exceeding the configured {maximum}-byte limit"
    )]
    BrokerResponseTooLarge { actual: usize, maximum: usize },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Browser operations stay independent of the current `Tool` trait, so callers can adapt their
/// richer outputs to the model provider without losing screenshots, structured snapshots, or file
/// references. This trait also makes server-side policy wrappers straightforward to mock in tests.
#[async_trait]
pub trait BrowserRuntime: Send + Sync {
    fn capabilities(&self) -> BrowserRuntimeCapabilities;
    async fn create_session(
        &self,
        spec: BrowserSessionSpec,
    ) -> Result<BrowserSessionInfo, BrowserError>;
    async fn grant_network_access(
        &self,
        session: BrowserSessionId,
        grant: BrowserNetworkGrant,
    ) -> Result<(), BrowserError>;
    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError>;
    async fn observe(
        &self,
        session: BrowserSessionId,
        options: BrowserObserveOptions,
    ) -> Result<BrowserObservation, BrowserError>;
    async fn switch_target(
        &self,
        session: BrowserSessionId,
        target: BrowserTargetRef,
    ) -> Result<BrowserOutput, BrowserError>;
    async fn observation_node(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
    ) -> Result<BrowserNode, BrowserError>;
    async fn perform(
        &self,
        session: BrowserSessionId,
        observation_id: BrowserObservationId,
        node_ref: BrowserNodeRef,
        action: BrowserAction,
    ) -> Result<BrowserActionReceipt, BrowserError>;
    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError>;
    async fn wait(
        &self,
        session: BrowserSessionId,
        request: BrowserWaitRequest,
    ) -> Result<BrowserOutput, BrowserError>;
    async fn download(
        &self,
        session: BrowserSessionId,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError>;
    async fn close_session(&self, session: BrowserSessionId) -> Result<(), BrowserError>;
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserContent, BrowserError, BrowserNetworkGrant, BrowserProfileId, BrowserSelector,
        MAX_BROWSER_PROFILE_ID_LEN, MAX_NETWORK_HOSTS,
    };
    use serde_json::json;

    #[test]
    fn selector_rejects_empty_values() {
        assert!(BrowserSelector::new("  ").is_err());
        assert_eq!(
            BrowserSelector::new("button.submit").unwrap().as_str(),
            "button.submit"
        );
    }

    #[test]
    fn browser_profile_ids_are_safe_and_serde_validated() {
        let profile = BrowserProfileId::new("team.release_1").unwrap();
        assert_eq!(profile.as_str(), "team.release_1");
        for invalid in ["", "-leading", "contains:colon", "contains/slash"] {
            assert!(matches!(
                BrowserProfileId::new(invalid),
                Err(BrowserError::InvalidProfileId(_))
            ));
            assert!(serde_json::from_value::<BrowserProfileId>(json!(invalid)).is_err());
        }
        assert!(BrowserProfileId::new("a".repeat(MAX_BROWSER_PROFILE_ID_LEN + 1)).is_err());
    }

    #[test]
    fn network_grants_normalize_and_deduplicate_exact_hosts() {
        let grant = BrowserNetworkGrant::new(["Example.COM.", "example.com", "127.0.0.1"])
            .expect("valid network grant");
        assert_eq!(grant.allowed_hosts, vec!["127.0.0.1", "example.com"]);
        assert_eq!(
            BrowserNetworkGrant::new(["::1"]).unwrap().allowed_hosts,
            vec!["::1"]
        );
        assert!(BrowserNetworkGrant::new(["example.com:443"]).is_err());
        assert!(BrowserNetworkGrant::new(["https://example.com"]).is_err());

        let maximum = (0..MAX_NETWORK_HOSTS)
            .map(|index| format!("host-{index}.example"))
            .collect::<Vec<_>>();
        let mut cumulative = BrowserNetworkGrant::new(maximum).unwrap();
        assert!(matches!(
            cumulative.merge(BrowserNetworkGrant::new(["overflow.example"]).unwrap()),
            Err(BrowserError::InvalidNetworkGrant(_))
        ));
        assert!(matches!(
            BrowserNetworkGrant::new(
                (0..=MAX_NETWORK_HOSTS).map(|index| format!("host-{index}.example"))
            ),
            Err(BrowserError::InvalidNetworkGrant(_))
        ));
    }

    #[test]
    fn browser_image_bytes_accept_compact_base64_and_legacy_arrays() {
        for value in [
            json!({ "type": "image", "mime_type": "image/png", "bytes": "iVBORw==" }),
            json!({ "type": "image", "mime_type": "image/png", "bytes": [137, 80, 78, 71] }),
        ] {
            let content: BrowserContent = serde_json::from_value(value).unwrap();
            assert!(matches!(
                content,
                BrowserContent::Image { bytes, .. } if bytes == b"\x89PNG"
            ));
        }
    }
}
