//! A bounded Chromium runtime with one shared browser profile and task-scoped target groups.
//!
//! This module intentionally stops at the browser boundary: callers decide whether a URL or an
//! interaction needs approval, while this runtime owns the browser process and its per-session
//! profile. The `BrowserContent` enum is a richer result contract than the current text-only tool
//! result and can be adapted to a future multimodal message protocol without re-reading data.

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::de::{SeqAccess, Visitor};
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::Cursor;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::ORIGIN, HeaderValue},
        Message as WebSocketMessage,
    },
    MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

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

const DEFAULT_BROWSER_PROFILE_ID: &str = "default";
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

    fn merge(&mut self, other: Self) -> Result<(), BrowserError> {
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
    fn new() -> Self {
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
    fn new() -> Self {
        Self(Uuid::new_v4().to_string())
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
    fn new() -> Self {
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
    fn materially_differs_from(&self, other: &Self) -> bool {
        (self.x - other.x).abs() > MAX_NODE_POSITION_DRIFT
            || (self.y - other.y).abs() > MAX_NODE_POSITION_DRIFT
            || (self.width - other.width).abs() > MAX_NODE_POSITION_DRIFT
            || (self.height - other.height).abs() > MAX_NODE_POSITION_DRIFT
    }
}

impl BrowserNode {
    fn matches_current(&self, current: &Self) -> bool {
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

struct LocalBrowserProcess {
    child: Child,
    browser_websocket_url: String,
    download_dir: PathBuf,
}

fn browser_profile_storage_root(
    config: &BrowserRuntimeConfig,
    spec: &BrowserSessionSpec,
) -> PathBuf {
    if spec.profile_id.as_str() == DEFAULT_BROWSER_PROFILE_ID
        && spec.profile_persistence == BrowserProfilePersistence::Persistent
    {
        return config.data_root.clone();
    }
    let persistence_directory = match spec.profile_persistence {
        BrowserProfilePersistence::Persistent => "profiles",
        BrowserProfilePersistence::Ephemeral => "ephemeral",
    };
    config
        .data_root
        .join(persistence_directory)
        .join(spec.profile_id.as_str())
}

impl LocalBrowserProcess {
    async fn start(
        config: Arc<BrowserRuntimeConfig>,
        spec: &BrowserSessionSpec,
    ) -> Result<Self, BrowserError> {
        let executable = discover_browser_executable(config.executable.as_deref())?;
        let storage_root = browser_profile_storage_root(&config, spec);
        let profile_dir = storage_root.join("profile");
        let download_dir = storage_root.join("downloads");
        tokio::fs::create_dir_all(&profile_dir).await?;
        tokio::fs::create_dir_all(&download_dir).await?;

        let mut command = Command::new(executable);
        command
            .arg("--remote-debugging-address=127.0.0.1")
            .arg("--remote-debugging-port=0")
            .arg("--remote-allow-origins=*")
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-networking")
            .arg("--disable-component-update")
            .arg("--disable-sync")
            .arg("--disable-extensions")
            .arg("--disable-popup-blocking")
            .arg("--disable-features=Translate,MediaRouter")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENTOPIA_API_KEY")
            .env_remove("OPENTOPIA_API_TOKEN");
        if config.headless {
            command.arg("--headless=new").arg("--disable-gpu");
        }
        command.arg("about:blank");
        let child = command.spawn()?;
        let port = match wait_for_devtools_port(&profile_dir, config.startup_timeout).await {
            Ok(port) => port,
            Err(error) => {
                let mut child = child;
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let browser_ws_url = match browser_websocket_url(port, config.startup_timeout).await {
            Ok(url) => url,
            Err(error) => {
                let mut child = child;
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let download_configuration = async {
            let browser = CdpPage::connect(&browser_ws_url, config.command_timeout)
                .await
                .map_err(|error| {
                    BrowserError::Protocol(format!(
                        "connecting to the browser DevTools endpoint: {error}"
                    ))
                })?;
            browser
                .command(
                    "Browser.setDownloadBehavior",
                    json!({
                        "behavior": "allow",
                        "downloadPath": download_dir,
                        "eventsEnabled": true,
                    }),
                )
                .await
                .map_err(|error| {
                    BrowserError::Protocol(format!(
                        "configuring the shared browser download directory: {error}"
                    ))
                })
        }
        .await;
        if let Err(error) = download_configuration {
            let mut child = child;
            let _ = child.kill().await;
            return Err(error);
        }

        Ok(Self {
            child,
            browser_websocket_url: browser_ws_url,
            download_dir,
        })
    }

    async fn shutdown(&mut self) -> Result<(), BrowserError> {
        match tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await {
            Ok(result) => {
                let _ = result?;
            }
            Err(_) => {
                self.child.kill().await?;
            }
        }
        Ok(())
    }
}

impl Drop for LocalBrowserProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
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
        targets.sort_by(|left, right| left.target_ref.0.cmp(&right.target_ref.0));
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

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum CdpCommand {
    Execute {
        client_id: u64,
        method: String,
        params: Value,
        session_id: Option<String>,
        response: oneshot::Sender<Result<Value, BrowserError>>,
    },
    Cancel {
        client_id: u64,
    },
}

#[derive(Debug, Clone)]
struct CdpEvent {
    method: String,
    params: Value,
    session_id: Option<String>,
}

struct CdpPage {
    commands: mpsc::Sender<CdpCommand>,
    events: broadcast::Receiver<CdpEvent>,
    buffered_events: Vec<CdpEvent>,
    command_timeout: Duration,
    session_id: Option<String>,
    target_id: Option<String>,
    next_client_id: Arc<AtomicU64>,
    network_policy: Arc<StdRwLock<Option<HashSet<String>>>>,
    connected: Arc<AtomicBool>,
}

impl CdpPage {
    async fn connect(websocket_url: &str, command_timeout: Duration) -> Result<Self, BrowserError> {
        let mut request = websocket_url
            .into_client_request()
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        request
            .headers_mut()
            .insert(ORIGIN, HeaderValue::from_static("http://localhost"));
        let (socket, _) = tokio::time::timeout(command_timeout, connect_async(request))
            .await
            .map_err(|_| BrowserError::Timeout("connecting to the local browser".to_string()))?
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        let (write, read) = socket.split();
        let (commands, command_rx) = mpsc::channel(64);
        let (event_tx, events) = broadcast::channel(512);
        let network_policy = Arc::new(StdRwLock::new(None));
        let connected = Arc::new(AtomicBool::new(true));
        tokio::spawn(run_cdp_connection(
            write,
            read,
            command_rx,
            event_tx,
            network_policy.clone(),
            connected.clone(),
        ));
        Ok(Self {
            commands,
            events,
            buffered_events: Vec::new(),
            command_timeout,
            session_id: None,
            target_id: None,
            next_client_id: Arc::new(AtomicU64::new(0)),
            network_policy,
            connected,
        })
    }

    async fn connect_chrome_bridge(
        bridge_url: &str,
        bridge_token: &str,
        browser_session_id: BrowserSessionId,
        command_timeout: Duration,
    ) -> Result<Self, BrowserError> {
        let base_url = validate_chrome_bridge_url(bridge_url)?;
        let authorization =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", bridge_token.trim()))
                .map_err(|_| {
                    BrowserError::BrokerConfiguration("Chrome bridge token is invalid".to_string())
                })?;
        if bridge_token.trim().is_empty() {
            return Err(BrowserError::BrokerConfiguration(
                "Chrome bridge token is empty".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(command_timeout.min(Duration::from_secs(5)))
            .build()?;
        let session_url = base_url
            .join(&format!("v1/backend/sessions/{browser_session_id}"))
            .map_err(|_| BrowserError::Protocol("Invalid Chrome session endpoint".to_string()))?;
        let response = tokio::time::timeout(
            command_timeout,
            client
                .get(session_url)
                .header(reqwest::header::AUTHORIZATION, authorization.clone())
                .send(),
        )
        .await
        .map_err(|_| BrowserError::Timeout("Chrome session attach".to_string()))?
        .map_err(map_chrome_bridge_transport_error)?;
        let status = response.status();
        let metadata = read_chrome_bridge_json(response).await?;
        if !status.is_success() {
            return Err(chrome_bridge_rejected(status.as_u16(), &metadata));
        }
        let target_id = metadata
            .get("targetId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                BrowserError::Protocol("Chrome bridge returned no target ID".to_string())
            })?
            .to_string();

        let (commands, command_rx) = mpsc::channel(64);
        let (event_tx, events) = broadcast::channel(512);
        let network_policy = Arc::new(StdRwLock::new(None));
        let connected = Arc::new(AtomicBool::new(true));
        tokio::spawn(run_chrome_bridge_commands(
            client.clone(),
            base_url.clone(),
            authorization.clone(),
            browser_session_id,
            command_timeout,
            command_rx,
            connected.clone(),
        ));
        tokio::spawn(run_chrome_bridge_events(
            client,
            base_url,
            authorization,
            browser_session_id,
            event_tx,
            connected.clone(),
        ));
        Ok(Self {
            commands,
            events,
            buffered_events: Vec::new(),
            command_timeout,
            session_id: Some("root".to_string()),
            target_id: Some(target_id),
            next_client_id: Arc::new(AtomicU64::new(0)),
            network_policy,
            connected,
        })
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn grant_network_access(&self, grant: BrowserNetworkGrant) -> Result<(), BrowserError> {
        let mut policy = self.network_policy.write().map_err(|_| {
            BrowserError::Protocol("Browser network policy lock is poisoned".into())
        })?;
        let allowed_hosts = policy.get_or_insert_with(HashSet::new);
        for host in grant.allowed_hosts {
            allowed_hosts.insert(normalize_network_host(&host)?);
        }
        Ok(())
    }

    async fn create_and_attach_target(&mut self) -> Result<(), BrowserError> {
        let target = self
            .command("Target.createTarget", json!({ "url": "about:blank" }))
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                BrowserError::Protocol("Target.createTarget returned no target ID".to_string())
            })?;
        let attached = self
            .command(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        self.target_id = Some(target_id.to_string());
        self.session_id = Some(
            attached
                .get("sessionId")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    BrowserError::Protocol(
                        "Target.attachToTarget returned no session ID".to_string(),
                    )
                })?
                .to_string(),
        );
        self.discard_events();
        Ok(())
    }

    async fn initialize_page_domains(&mut self) -> Result<(), BrowserError> {
        let session_id = self.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("No active CDP page session is attached".to_string())
        })?;
        self.initialize_page_domains_for_session(&session_id).await
    }

    async fn initialize_external_page_domains(&mut self) -> Result<(), BrowserError> {
        let session_id = self.session_id.clone().ok_or_else(|| {
            BrowserError::Protocol("No active Chrome page session is attached".to_string())
        })?;
        self.initialize_external_page_domains_for_session(&session_id)
            .await
    }

    async fn initialize_external_page_domains_for_session(
        &self,
        session_id: &str,
    ) -> Result<(), BrowserError> {
        let session_id = Some(session_id.to_string());
        self.command_for_session("Page.enable", json!({}), session_id.clone())
            .await?;
        self.command_for_session("Runtime.enable", json!({}), session_id.clone())
            .await?;
        self.command_for_session("Network.enable", json!({}), session_id.clone())
            .await?;
        self.command_for_session(
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
            session_id,
        )
        .await
        .map(|_| ())
    }

    async fn initialize_page_domains_for_session(
        &self,
        session_id: &str,
    ) -> Result<(), BrowserError> {
        let session_id = Some(session_id.to_string());
        self.command_for_session("Page.enable", json!({}), session_id.clone())
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("enabling the Page domain: {error}"))
            })?;
        self.command_for_session("Runtime.enable", json!({}), session_id.clone())
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("enabling the Runtime domain: {error}"))
            })?;
        self.command_for_session("Network.enable", json!({}), session_id.clone())
            .await
            .map_err(|error| {
                BrowserError::Protocol(format!("enabling the Network domain: {error}"))
            })?;
        self.command_for_session(
            "Fetch.enable",
            json!({ "patterns": [{ "urlPattern": "http://*/*", "requestStage": "Request" }, { "urlPattern": "https://*/*", "requestStage": "Request" }] }),
            session_id.clone(),
        )
        .await
        .map_err(|error| {
            BrowserError::Protocol(format!("enabling request interception: {error}"))
        })?;
        self.command_for_session(
            "Page.setLifecycleEventsEnabled",
            json!({ "enabled": true }),
            session_id,
        )
        .await
        .map(|_| ())
    }

    async fn enable_target_discovery(&self) -> Result<(), BrowserError> {
        self.root_command("Target.setDiscoverTargets", json!({ "discover": true }))
            .await?;
        self.enable_target_auto_attach().await
    }

    async fn enable_target_auto_attach(&self) -> Result<(), BrowserError> {
        self.root_command(
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": false,
                "flatten": true,
                "filter": [
                    { "type": "page", "exclude": false },
                    { "exclude": true }
                ]
            }),
        )
        .await
        .map(|_| ())
    }

    fn activate(&mut self, target_id: String, session_id: String) {
        self.target_id = Some(target_id);
        self.session_id = Some(session_id);
    }

    async fn configure_downloads(&self, download_dir: &Path) -> Result<(), BrowserError> {
        self.root_command(
            "Browser.setDownloadBehavior",
            json!({
                "behavior": "allow",
                "downloadPath": download_dir,
                "eventsEnabled": true,
            }),
        )
        .await
        .map(|_| ())
    }

    async fn command(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        self.command_for_session(method, params, self.session_id.clone())
            .await
    }

    async fn root_command(&self, method: &str, params: Value) -> Result<Value, BrowserError> {
        self.command_for_session(method, params, None).await
    }

    async fn command_for_session(
        &self,
        method: &str,
        params: Value,
        session_id: Option<String>,
    ) -> Result<Value, BrowserError> {
        let (response, receiver) = oneshot::channel();
        let client_id = self
            .next_client_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        self.commands
            .send(CdpCommand::Execute {
                client_id,
                method: method.to_string(),
                params,
                session_id,
                response,
            })
            .await
            .map_err(|_| {
                BrowserError::Disconnected("DevTools command channel closed".to_string())
            })?;
        match tokio::time::timeout(self.command_timeout, receiver).await {
            Ok(response) => response.map_err(|_| {
                BrowserError::Disconnected("DevTools response channel closed".to_string())
            })?,
            Err(_) => {
                let _ = self.commands.try_send(CdpCommand::Cancel { client_id });
                Err(BrowserError::Timeout(method.to_string()))
            }
        }
    }

    fn discard_events(&mut self) {
        self.buffered_events.clear();
        loop {
            match self.events.try_recv() {
                Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(broadcast::error::TryRecvError::Empty)
                | Err(broadcast::error::TryRecvError::Closed) => break,
            }
        }
    }

    fn try_next_event(&mut self) -> Result<Option<CdpEvent>, BrowserError> {
        while let Some(event) = self.buffered_events.pop() {
            if self.event_belongs_to_session(&event) {
                return Ok(Some(event));
            }
        }
        loop {
            match self.events.try_recv() {
                Ok(event) if self.event_belongs_to_session(&event) => return Ok(Some(event)),
                Ok(_) => continue,
                Err(broadcast::error::TryRecvError::Empty) => return Ok(None),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => {
                    return Err(BrowserError::Disconnected(
                        "Browser closed the DevTools event stream".to_string(),
                    ));
                }
            }
        }
    }

    async fn next_event(&mut self) -> Result<Option<CdpEvent>, BrowserError> {
        loop {
            match self.events.recv().await {
                Ok(event) if self.event_belongs_to_session(&event) => return Ok(Some(event)),
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(BrowserError::Disconnected(
                        "Browser closed the DevTools event stream".to_string(),
                    ));
                }
            }
        }
    }

    fn push_event(&mut self, event: CdpEvent) {
        self.buffered_events.push(event);
    }

    fn event_belongs_to_session(&self, event: &CdpEvent) -> bool {
        event.session_id.as_deref() == self.session_id.as_deref()
            || (event.session_id.is_none()
                && (event.method.starts_with("Browser.download")
                    || event.method.starts_with("Target.")))
    }
}

async fn run_cdp_connection(
    mut write: futures_util::stream::SplitSink<CdpSocket, WebSocketMessage>,
    mut read: futures_util::stream::SplitStream<CdpSocket>,
    mut commands: mpsc::Receiver<CdpCommand>,
    events: broadcast::Sender<CdpEvent>,
    network_policy: Arc<StdRwLock<Option<HashSet<String>>>>,
    connected: Arc<AtomicBool>,
) {
    let mut next_id = 0_u64;
    let mut pending =
        HashMap::<u64, (u64, String, oneshot::Sender<Result<Value, BrowserError>>)>::new();
    let disconnect_reason = loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    break "DevTools command channel closed".to_string();
                };
                pending.retain(|_, (_, _, response)| !response.is_closed());
                match command {
                    CdpCommand::Cancel { client_id } => {
                        pending.retain(|_, (pending_client_id, _, _)| *pending_client_id != client_id);
                    }
                    CdpCommand::Execute { client_id, method, params, session_id, response } => {
                        next_id = next_id.saturating_add(1);
                        let id = next_id;
                        let mut message = json!({ "id": id, "method": method, "params": params });
                        if let Some(session_id) = session_id.as_ref() {
                            message["sessionId"] = Value::String(session_id.clone());
                        }
                        if let Err(error) = write.send(WebSocketMessage::Text(message.to_string())).await {
                            let reason = error.to_string();
                            let _ = response.send(Err(BrowserError::Disconnected(reason.clone())));
                            break reason;
                        }
                        pending.insert(id, (client_id, method, response));
                    }
                }
            }
            incoming = read.next() => {
                let Some(incoming) = incoming else {
                    break "Browser closed the DevTools connection".to_string();
                };
                match incoming {
                    Ok(WebSocketMessage::Text(text)) => {
                        let message = match serde_json::from_str::<Value>(&text) {
                            Ok(message) => message,
                            Err(error) => break format!("Invalid DevTools message: {error}"),
                        };
                        if let Some(id) = message.get("id").and_then(Value::as_u64) {
                            if let Some((_, method, response)) = pending.remove(&id) {
                                let result = if let Some(error) = message.get("error") {
                                    Err(BrowserError::Cdp {
                                        method,
                                        message: error
                                            .get("message")
                                            .and_then(Value::as_str)
                                            .unwrap_or("Unknown DevTools error")
                                            .to_string(),
                                    })
                                } else {
                                    Ok(message.get("result").cloned().unwrap_or(Value::Null))
                                };
                                let _ = response.send(result);
                            }
                        } else if let Some(method) = message.get("method").and_then(Value::as_str) {
                            if method == "Target.attachedToTarget" {
                                if let Some(attached_session_id) = message
                                    .pointer("/params/sessionId")
                                    .and_then(Value::as_str)
                                {
                                    next_id = next_id.saturating_add(1);
                                    let resume = json!({
                                        "id": next_id,
                                        "method": "Runtime.runIfWaitingForDebugger",
                                        "params": {},
                                        "sessionId": attached_session_id,
                                    });
                                    if let Err(error) = write
                                        .send(WebSocketMessage::Text(resume.to_string()))
                                        .await
                                    {
                                        break error.to_string();
                                    }
                                }
                            }
                            if method == "Page.javascriptDialogOpening" {
                                next_id = next_id.saturating_add(1);
                                let mut dismissal = json!({
                                    "id": next_id,
                                    "method": "Page.handleJavaScriptDialog",
                                    "params": { "accept": false },
                                });
                                if let Some(session_id) = message.get("sessionId").and_then(Value::as_str) {
                                    dismissal["sessionId"] = Value::String(session_id.to_string());
                                }
                                if let Err(error) = write
                                    .send(WebSocketMessage::Text(dismissal.to_string()))
                                    .await
                                {
                                    break error.to_string();
                                }
                            }
                            if method == "Fetch.requestPaused" {
                                let params = message.get("params").cloned().unwrap_or(Value::Null);
                                let request_id = params
                                    .get("requestId")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let request_url = params
                                    .pointer("/request/url")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let allowed = network_policy_allows_url(&network_policy, request_url);
                                next_id = next_id.saturating_add(1);
                                let interception_method = if allowed {
                                    "Fetch.continueRequest"
                                } else {
                                    "Fetch.failRequest"
                                };
                                let interception_params = if allowed {
                                    json!({ "requestId": request_id })
                                } else {
                                    json!({ "requestId": request_id, "errorReason": "BlockedByClient" })
                                };
                                let mut interception = json!({
                                    "id": next_id,
                                    "method": interception_method,
                                    "params": interception_params,
                                });
                                if let Some(session_id) = message.get("sessionId").and_then(Value::as_str) {
                                    interception["sessionId"] = Value::String(session_id.to_string());
                                }
                                if let Err(error) = write
                                    .send(WebSocketMessage::Text(interception.to_string()))
                                    .await
                                {
                                    break error.to_string();
                                }
                                if !allowed {
                                    let _ = events.send(CdpEvent {
                                        method: "OpenTopia.networkRequestBlocked".to_string(),
                                        params: json!({
                                            "url": request_url,
                                            "host": network_request_host(request_url),
                                            "resourceType": params.get("resourceType").cloned(),
                                        }),
                                        session_id: message
                                            .get("sessionId")
                                            .and_then(Value::as_str)
                                            .map(str::to_string),
                                    });
                                }
                                continue;
                            }
                            let _ = events.send(CdpEvent {
                                method: method.to_string(),
                                params: message.get("params").cloned().unwrap_or(Value::Null),
                                session_id: message
                                    .get("sessionId")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            });
                        }
                    }
                    Ok(WebSocketMessage::Ping(payload)) => {
                        if let Err(error) = write.send(WebSocketMessage::Pong(payload)).await {
                            break error.to_string();
                        }
                    }
                    Ok(WebSocketMessage::Close(_)) => {
                        break "Browser closed the DevTools connection".to_string();
                    }
                    Ok(WebSocketMessage::Binary(_))
                    | Ok(WebSocketMessage::Pong(_))
                    | Ok(WebSocketMessage::Frame(_)) => {}
                    Err(error) => break error.to_string(),
                }
            }
        }
    };

    connected.store(false, Ordering::Release);
    for (_, (_, _, response)) in pending {
        let _ = response.send(Err(BrowserError::Disconnected(disconnect_reason.clone())));
    }
}

async fn run_chrome_bridge_commands(
    client: reqwest::Client,
    base_url: reqwest::Url,
    authorization: reqwest::header::HeaderValue,
    browser_session_id: BrowserSessionId,
    command_timeout: Duration,
    mut commands: mpsc::Receiver<CdpCommand>,
    connected: Arc<AtomicBool>,
) {
    let endpoint = match base_url.join("v1/backend/command") {
        Ok(endpoint) => endpoint,
        Err(_) => {
            connected.store(false, Ordering::Release);
            return;
        }
    };
    while let Some(command) = commands.recv().await {
        match command {
            CdpCommand::Cancel { .. } => {}
            CdpCommand::Execute {
                method,
                params,
                session_id,
                response,
                ..
            } => {
                let target_session_id = session_id.unwrap_or_else(|| "root".to_string());
                let request = client
                    .post(endpoint.clone())
                    .header(reqwest::header::AUTHORIZATION, authorization.clone())
                    .json(&json!({
                        "sessionId": browser_session_id,
                        "targetSessionId": target_session_id,
                        "method": method,
                        "params": params,
                    }));
                let result = match tokio::time::timeout(command_timeout, request.send()).await {
                    Err(_) => Err(BrowserError::Timeout(method)),
                    Ok(Err(error)) => Err(map_chrome_bridge_transport_error(error)),
                    Ok(Ok(http_response)) => {
                        let status = http_response.status();
                        match read_chrome_bridge_json(http_response).await {
                            Ok(value) if status.is_success() => {
                                Ok(value.get("result").cloned().unwrap_or(Value::Null))
                            }
                            Ok(value) => Err(chrome_bridge_rejected(status.as_u16(), &value)),
                            Err(error) => Err(error),
                        }
                    }
                };
                if matches!(result, Err(BrowserError::BrokerUnavailable)) {
                    connected.store(false, Ordering::Release);
                }
                let _ = response.send(result);
            }
        }
    }
    connected.store(false, Ordering::Release);
}

async fn run_chrome_bridge_events(
    client: reqwest::Client,
    base_url: reqwest::Url,
    authorization: reqwest::header::HeaderValue,
    browser_session_id: BrowserSessionId,
    events: broadcast::Sender<CdpEvent>,
    connected: Arc<AtomicBool>,
) {
    let endpoint = match base_url.join(&format!("v1/backend/events/{browser_session_id}")) {
        Ok(endpoint) => endpoint,
        Err(_) => {
            connected.store(false, Ordering::Release);
            return;
        }
    };
    let mut after = 0_u64;
    while connected.load(Ordering::Acquire) {
        let response = client
            .get(endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, authorization.clone())
            .query(&[
                ("after", after.to_string()),
                ("waitMs", "25000".to_string()),
            ])
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                connected.store(false, Ordering::Release);
                break;
            }
        };
        let status = response.status();
        let payload = match read_chrome_bridge_json(response).await {
            Ok(payload) if status.is_success() => payload,
            _ => {
                connected.store(false, Ordering::Release);
                break;
            }
        };
        if payload.get("attached").and_then(Value::as_bool) == Some(false) {
            connected.store(false, Ordering::Release);
            break;
        }
        for event in payload
            .get("events")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(sequence) = event.get("seq").and_then(Value::as_u64) else {
                continue;
            };
            let Some(method) = event.get("method").and_then(Value::as_str) else {
                continue;
            };
            after = after.max(sequence);
            let _ = events.send(CdpEvent {
                method: method.to_string(),
                params: event.get("params").cloned().unwrap_or(Value::Null),
                session_id: event
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }
}

pub(crate) fn validate_chrome_bridge_url(raw: &str) -> Result<reqwest::Url, BrowserError> {
    let mut url = reqwest::Url::parse(raw.trim()).map_err(|_| {
        BrowserError::BrokerConfiguration("Chrome bridge URL is invalid".to_string())
    })?;
    if url.scheme() != "http" || !url.username().is_empty() || url.password().is_some() {
        return Err(BrowserError::BrokerConfiguration(
            "Chrome bridge URL must be credential-free HTTP".to_string(),
        ));
    }
    let loopback = url
        .host_str()
        .and_then(|host| host.parse::<IpAddr>().ok())
        .is_some_and(|address| address.is_loopback());
    if !loopback || url.query().is_some() || url.fragment().is_some() {
        return Err(BrowserError::BrokerConfiguration(
            "Chrome bridge URL must use a numeric loopback address".to_string(),
        ));
    }
    url.set_path(&format!("{}/", url.path().trim_end_matches('/')));
    Ok(url)
}

async fn read_chrome_bridge_json(response: reqwest::Response) -> Result<Value, BrowserError> {
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > MAX_CHROME_BRIDGE_RESPONSE_BYTES as u64) {
        return Err(BrowserError::BrokerResponseTooLarge {
            actual: content_length.unwrap_or_default() as usize,
            maximum: MAX_CHROME_BRIDGE_RESPONSE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(content_length.unwrap_or_default() as usize);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(map_chrome_bridge_transport_error)?;
        let actual = bytes.len().saturating_add(chunk.len());
        if actual > MAX_CHROME_BRIDGE_RESPONSE_BYTES {
            return Err(BrowserError::BrokerResponseTooLarge {
                actual,
                maximum: MAX_CHROME_BRIDGE_RESPONSE_BYTES,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| BrowserError::Protocol("Chrome bridge returned invalid JSON".to_string()))
}

fn chrome_bridge_rejected(status: u16, body: &Value) -> BrowserError {
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or("Chrome bridge rejected the request")
        .chars()
        .take(512)
        .collect();
    BrowserError::BrokerRejected { status, message }
}

fn map_chrome_bridge_transport_error(error: reqwest::Error) -> BrowserError {
    if error.is_timeout() {
        BrowserError::Timeout("Chrome bridge response".to_string())
    } else {
        BrowserError::BrokerUnavailable
    }
}

fn normalize_network_host(raw_host: &str) -> Result<String, BrowserError> {
    let host = raw_host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host.contains('/')
        || host.contains('@')
        || host.contains(char::is_whitespace)
    {
        return Err(BrowserError::InvalidUrl(format!(
            "invalid network host `{raw_host}`"
        )));
    }
    let parsed = reqwest::Url::parse(&format!("http://[{host}]/"))
        .or_else(|_| reqwest::Url::parse(&format!("http://{host}/")))
        .map_err(|_| BrowserError::InvalidUrl(format!("invalid network host `{raw_host}`")))?;
    if parsed.port().is_some() {
        return Err(BrowserError::InvalidUrl(format!(
            "network host must not include a port: `{raw_host}`"
        )));
    }
    parsed
        .host_str()
        .map(|host| host.trim_matches(['[', ']']).to_ascii_lowercase())
        .ok_or_else(|| BrowserError::InvalidUrl(format!("invalid network host `{raw_host}`")))
}

fn network_request_host(raw_url: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw_url).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.host_str()
        .and_then(|host| normalize_network_host(host).ok())
}

fn network_policy_allows_url(policy: &StdRwLock<Option<HashSet<String>>>, raw_url: &str) -> bool {
    let Ok(policy) = policy.read() else {
        return false;
    };
    let Some(allowed_hosts) = policy.as_ref() else {
        return true;
    };
    network_request_host(raw_url).is_some_and(|host| allowed_hosts.contains(&host))
}

fn png_looks_blank(bytes: &[u8]) -> Result<bool, BrowserError> {
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|error| BrowserError::Protocol(format!("Invalid screenshot PNG: {error}")))?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| BrowserError::Protocol(format!("Invalid screenshot PNG: {error}")))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Ok(false);
    }
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Ok(false),
    };
    let pixels = &pixels[..info.buffer_size()];
    let pixel_count = pixels.len() / channels;
    if pixel_count == 0 {
        return Ok(true);
    }
    let stride = (pixel_count / 4096).max(1);
    let mut sampled = 0_usize;
    let mut blank = 0_usize;
    for pixel_index in (0..pixel_count).step_by(stride) {
        let pixel = &pixels[pixel_index * channels..][..channels];
        let (red, green, blue, alpha) = match info.color_type {
            png::ColorType::Grayscale => (pixel[0], pixel[0], pixel[0], 255),
            png::ColorType::GrayscaleAlpha => (pixel[0], pixel[0], pixel[0], pixel[1]),
            png::ColorType::Rgb => (pixel[0], pixel[1], pixel[2], 255),
            png::ColorType::Rgba => (pixel[0], pixel[1], pixel[2], pixel[3]),
            png::ColorType::Indexed => unreachable!(),
        };
        sampled += 1;
        if alpha <= 2 || (red <= 3 && green <= 3 && blue <= 3) {
            blank += 1;
        }
    }
    Ok(blank as f64 / sampled as f64 >= 0.995)
}

async fn wait_for_devtools_port(
    profile_dir: &Path,
    timeout: Duration,
) -> Result<u16, BrowserError> {
    let started = tokio::time::Instant::now();
    let active_port_file = profile_dir.join("DevToolsActivePort");
    loop {
        if let Ok(contents) = tokio::fs::read_to_string(&active_port_file).await {
            if let Some(port) = contents
                .lines()
                .next()
                .and_then(|value| value.parse::<u16>().ok())
            {
                return Ok(port);
            }
        }
        if started.elapsed() >= timeout {
            return Err(BrowserError::StartupTimeout(timeout));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn browser_websocket_url(port: u16, timeout: Duration) -> Result<String, BrowserError> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let endpoint = format!("http://127.0.0.1:{port}/json/version");
    let started = tokio::time::Instant::now();
    loop {
        if let Ok(response) = client.get(&endpoint).send().await {
            if let Ok(target) = response.json::<Value>().await {
                if let Some(websocket_url) =
                    target.get("webSocketDebuggerUrl").and_then(Value::as_str)
                {
                    return Ok(websocket_url.to_string());
                }
            }
        }
        if started.elapsed() >= timeout {
            return Err(BrowserError::StartupTimeout(timeout));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn list_downloads(directory: &Path) -> Result<HashSet<PathBuf>, BrowserError> {
    let mut entries = tokio::fs::read_dir(directory).await?;
    let mut paths = HashSet::new();
    while let Some(entry) = entries.next_entry().await? {
        paths.insert(entry.path());
    }
    Ok(paths)
}

async fn wait_for_download(
    page: &mut CdpPage,
    directory: &Path,
    before: &HashSet<PathBuf>,
    expected_filename: Option<&str>,
    timeout: Duration,
    maximum_bytes: u64,
) -> Result<BrowserDownload, BrowserError> {
    let started = tokio::time::Instant::now();
    let mut last_candidate: Option<(PathBuf, u64)> = None;
    let mut download_guid = None;
    let mut protocol_completed = false;
    loop {
        while let Some(event) = page.try_next_event()? {
            match event.method.as_str() {
                "Browser.downloadWillBegin" => {
                    download_guid = event
                        .params
                        .get("guid")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
                "Browser.downloadProgress" => {
                    let event_guid = event.params.get("guid").and_then(Value::as_str);
                    if download_guid
                        .as_deref()
                        .is_none_or(|guid| event_guid == Some(guid))
                    {
                        let received = event
                            .params
                            .get("receivedBytes")
                            .and_then(Value::as_f64)
                            .unwrap_or_default()
                            .max(0.0) as u64;
                        if received > maximum_bytes {
                            cancel_browser_download(page, download_guid.as_deref()).await;
                            return Err(BrowserError::DownloadTooLarge {
                                maximum: maximum_bytes,
                            });
                        }
                        match event.params.get("state").and_then(Value::as_str) {
                            Some("completed") => protocol_completed = true,
                            Some("canceled") => {
                                return Err(BrowserError::Protocol(
                                    "Browser download was canceled".to_string(),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if before.contains(&path) {
                continue;
            }
            let metadata = entry.metadata().await?;
            if !metadata.is_file() {
                continue;
            }
            let bytes = metadata.len();
            if bytes > maximum_bytes {
                cancel_browser_download(page, download_guid.as_deref()).await;
                return Err(BrowserError::DownloadTooLarge {
                    maximum: maximum_bytes,
                });
            }
            if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("crdownload" | "tmp")
            ) {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().to_string();
            if expected_filename.is_some_and(|expected| expected != filename) {
                continue;
            }
            if protocol_completed || last_candidate.as_ref() == Some(&(path.clone(), bytes)) {
                return Ok(BrowserDownload {
                    content_type: content_type_for_path(&path),
                    path,
                    filename,
                    bytes,
                });
            }
            last_candidate = Some((path, bytes));
        }
        if started.elapsed() >= timeout {
            cancel_browser_download(page, download_guid.as_deref()).await;
            return Err(BrowserError::DownloadTimeout);
        }
        tokio::select! {
            event = page.next_event() => {
                if let Some(event) = event? {
                    page.push_event(event);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(150)) => {}
        }
    }
}

async fn cancel_browser_download(page: &CdpPage, guid: Option<&str>) {
    if let Some(guid) = guid {
        let _ = page
            .root_command("Browser.cancelDownload", json!({ "guid": guid }))
            .await;
    }
}

fn content_type_for_path(path: &Path) -> Option<String> {
    match path.extension().and_then(|extension| extension.to_str())? {
        "csv" => Some("text/csv".to_string()),
        "json" => Some("application/json".to_string()),
        "pdf" => Some("application/pdf".to_string()),
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "txt" | "log" => Some("text/plain".to_string()),
        "zip" => Some("application/zip".to_string()),
        _ => None,
    }
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

fn discover_browser_executable(configured: Option<&Path>) -> Result<PathBuf, BrowserError> {
    if let Some(configured) = configured {
        return configured
            .is_file()
            .then(|| configured.to_path_buf())
            .ok_or_else(|| BrowserError::ExecutableMissing(configured.to_path_buf()));
    }

    let mut candidates = Vec::new();
    for variable in ["OPENTOPIA_BROWSER_EXECUTABLE", "CHROME_PATH"] {
        if let Some(path) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(variable).map(PathBuf::from) {
                candidates.push(root.join("Google/Chrome/Application/chrome.exe"));
                candidates.push(root.join("Microsoft/Edge/Application/msedge.exe"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        for path in [
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/microsoft-edge",
        ] {
            candidates.push(PathBuf::from(path));
        }
    }

    let executable_names: &[&str] = if cfg!(windows) {
        &["chrome.exe", "msedge.exe"]
    } else {
        &["google-chrome", "chromium", "microsoft-edge"]
    };
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            for executable in executable_names {
                candidates.push(directory.join(executable));
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or(BrowserError::ExecutableNotFound)
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
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async;

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
    fn browser_profile_storage_preserves_legacy_default_and_isolates_named_profiles() {
        let config = BrowserRuntimeConfig {
            data_root: PathBuf::from("browser-data"),
            ..BrowserRuntimeConfig::default()
        };
        let session = BrowserSessionId::new();
        let default_spec = BrowserSessionSpec::from(session);
        assert_eq!(
            browser_profile_storage_root(&config, &default_spec),
            PathBuf::from("browser-data")
        );

        let named = BrowserSessionSpec::persistent(session, BrowserProfileId::new("work").unwrap());
        assert_eq!(
            browser_profile_storage_root(&config, &named),
            PathBuf::from("browser-data").join("profiles").join("work")
        );
        let ephemeral = BrowserSessionSpec {
            profile_persistence: BrowserProfilePersistence::Ephemeral,
            ..named
        };
        assert_eq!(
            browser_profile_storage_root(&config, &ephemeral),
            PathBuf::from("browser-data").join("ephemeral").join("work")
        );
    }

    #[test]
    fn local_runtime_advertises_managed_backend_guarantees() {
        let runtime = LocalBrowserRuntime::new(BrowserRuntimeConfig::default());
        let capabilities = runtime.capabilities();
        assert_eq!(capabilities.backend, BrowserBackendKind::LocalChrome);
        assert_eq!(capabilities.surface, BrowserSurfaceKind::Headless);
        assert!(capabilities.hard_network_isolation);
        assert!(!capabilities.supports_external_profile);
        assert!(capabilities
            .profile_persistence
            .contains(&BrowserProfilePersistence::Ephemeral));
    }

    #[test]
    fn url_validation_is_scheme_bounded() {
        let runtime = LocalBrowserRuntime::new(BrowserRuntimeConfig::default());
        assert!(runtime.validate_url("https://example.com/a").is_ok());
        assert!(matches!(
            runtime.validate_url("file:///etc/passwd"),
            Err(BrowserError::DisallowedScheme(_))
        ));
        assert!(matches!(
            runtime.validate_url("not a url"),
            Err(BrowserError::InvalidUrl(_))
        ));
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
    fn utf8_truncation_keeps_valid_boundaries() {
        let (value, truncated) = truncate_utf8("ab你好", 4);
        assert_eq!(value, "ab");
        assert!(truncated);
    }

    #[test]
    fn download_content_types_are_inferred_for_common_files() {
        assert_eq!(
            content_type_for_path(Path::new("report.pdf")),
            Some("application/pdf".to_string())
        );
        assert_eq!(content_type_for_path(Path::new("report.unknown")), None);
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

    #[test]
    fn screenshot_pixel_validation_distinguishes_black_from_rendered_content() {
        fn image(red: u8, green: u8, blue: u8) -> Vec<u8> {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                let pixel = [red, green, blue, 255];
                writer.write_image_data(&pixel.repeat(4)).unwrap();
            }
            bytes
        }

        assert!(png_looks_blank(&image(0, 0, 0)).unwrap());
        assert!(!png_looks_blank(&image(24, 120, 220)).unwrap());
    }

    #[tokio::test]
    async fn cdp_connection_correlates_responses_and_routes_session_events() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let command = socket.next().await.unwrap().unwrap();
            let WebSocketMessage::Text(command) = command else {
                panic!("expected a text command");
            };
            let command: Value = serde_json::from_str(&command).unwrap();
            assert_eq!(command["sessionId"], "session-a");
            socket
                .send(WebSocketMessage::Text(
                    json!({
                        "method": "Page.frameNavigated",
                        "sessionId": "session-b",
                        "params": { "frame": { "url": "https://ignored.example" } }
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
            socket
                .send(WebSocketMessage::Text(
                    json!({
                        "method": "Page.frameNavigated",
                        "sessionId": "session-a",
                        "params": { "frame": { "url": "https://example.com" } }
                    })
                    .to_string(),
                ))
                .await
                .unwrap();
            socket
                .send(WebSocketMessage::Text(
                    json!({ "id": command["id"], "sessionId": "session-a", "result": { "ok": true } })
                        .to_string(),
                ))
                .await
                .unwrap();
        });

        let mut page = CdpPage::connect(&format!("ws://{address}"), Duration::from_secs(2))
            .await
            .unwrap();
        page.session_id = Some("session-a".to_string());
        let response = page.command("Runtime.evaluate", json!({})).await.unwrap();
        assert_eq!(response["ok"], true);
        let event = page.next_event().await.unwrap().unwrap();
        assert_eq!(event.session_id.as_deref(), Some("session-a"));
        assert_eq!(
            event.params.pointer("/frame/url").and_then(Value::as_str),
            Some("https://example.com")
        );
        server.await.unwrap();
        for _ in 0..20 {
            if !page.is_connected() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!page.is_connected());
    }

    #[tokio::test]
    async fn cdp_connection_enforces_network_grants_inside_the_actor() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let command = socket.next().await.unwrap().unwrap();
            let WebSocketMessage::Text(command) = command else {
                panic!("expected a text command");
            };
            let command: Value = serde_json::from_str(&command).unwrap();

            for (request_id, url, expected_method) in [
                (
                    "allowed",
                    "https://example.com/app.js",
                    "Fetch.continueRequest",
                ),
                (
                    "blocked",
                    "https://tracker.example/pixel",
                    "Fetch.failRequest",
                ),
            ] {
                socket
                    .send(WebSocketMessage::Text(
                        json!({
                            "method": "Fetch.requestPaused",
                            "sessionId": "session-a",
                            "params": {
                                "requestId": request_id,
                                "request": { "url": url },
                                "resourceType": "Script"
                            }
                        })
                        .to_string(),
                    ))
                    .await
                    .unwrap();
                let interception = socket.next().await.unwrap().unwrap();
                let WebSocketMessage::Text(interception) = interception else {
                    panic!("expected a text interception command");
                };
                let interception: Value = serde_json::from_str(&interception).unwrap();
                assert_eq!(interception["method"], expected_method);
                assert_eq!(interception["params"]["requestId"], request_id);
                assert_eq!(interception["sessionId"], "session-a");
            }

            socket
                .send(WebSocketMessage::Text(
                    json!({ "id": command["id"], "result": { "ok": true } }).to_string(),
                ))
                .await
                .unwrap();
        });

        let mut page = CdpPage::connect(&format!("ws://{address}"), Duration::from_secs(2))
            .await
            .unwrap();
        page.session_id = Some("session-a".to_string());
        page.grant_network_access(BrowserNetworkGrant::new(["example.com"]).unwrap())
            .unwrap();
        let response = page.command("Runtime.evaluate", json!({})).await.unwrap();
        assert_eq!(response["ok"], true);
        let blocked = page.next_event().await.unwrap().unwrap();
        assert_eq!(blocked.method, "OpenTopia.networkRequestBlocked");
        assert_eq!(blocked.params["host"], "tracker.example");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn local_chromium_runtime_smoke_test() {
        if discover_browser_executable(None).is_err() {
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut request = [0_u8; 4096];
                    let read = socket.read(&mut request).await.unwrap_or_default();
                    let request = String::from_utf8_lossy(&request[..read]);
                    if request.starts_with("GET /redirect ") {
                        let response = format!(
                            "HTTP/1.1 302 Found\r\nLocation: http://localhost:{}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            address.port()
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                        return;
                    }
                    if request.starts_with("GET /large-download ") {
                        let body = vec![b'x'; 8 * 1024];
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=large.bin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.write_all(&body).await;
                        let _ = socket.shutdown().await;
                        return;
                    }
                    if request.starts_with("GET /frame ") {
                        let body = concat!(
                            "<html><head><title>Frame fixture</title></head><body>",
                            "<main id='frame-state'>Frame ready</main><div id='frame-host'></div>",
                            "<script>const r=document.querySelector('#frame-host').attachShadow({mode:'open'});",
                            "r.innerHTML=\"<button id='frame-action'>Frame shadow action</button>\";",
                            "r.querySelector('button').onclick=()=>document.querySelector('#frame-state').textContent='Frame shadow clicked';</script>",
                            "</body></html>"
                        );
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                        return;
                    }
                    if request.starts_with("GET /popup ") {
                        let body = "<html><head><title>Owned popup</title></head><body><main>Popup ready</main></body></html>";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                        return;
                    }
                    if request.starts_with("GET /complex ") {
                        let body = format!(
                            concat!(
                                "<html><head><title>Complex CDP fixture</title></head><body>",
                                "<main id='state'>Complex ready</main>",
                                "<select id='plan' onchange=\"document.querySelector('#state').textContent='selected:'+this.value\"><option value='basic'>Basic</option><option value='pro'>Professional</option></select>",
                                "<button id='hover' onmouseenter=\"document.querySelector('#state').textContent='hovered'\">Hover action</button>",
                                "<button id='popup' onclick=\"window.open('/popup','_blank')\">Open popup</button>",
                                "<button id='dialog' onclick=\"alert('fixture dialog');document.querySelector('#state').textContent='dialog handled'\">Show dialog</button>",
                                "<div id='shadow-host'></div>",
                                "<div id='scroller' tabindex='0' style='height:80px;overflow:auto' onscroll=\"document.querySelector('#scroll-state').textContent='scrolled'\"><div style='height:700px'></div><button id='offscreen'>Offscreen action</button></div><output id='scroll-state'>not scrolled</output>",
                                "<iframe src='/frame'></iframe><iframe src='http://localhost:{}/frame'></iframe>",
                                "<script>const a=document.querySelector('#shadow-host').attachShadow({{mode:'open'}});a.innerHTML=\"<section id='nested'></section>\";const b=a.querySelector('#nested').attachShadow({{mode:'open'}});b.innerHTML=\"<button>Nested shadow action</button>\";b.querySelector('button').onclick=()=>document.querySelector('#state').textContent='shadow clicked';</script>",
                                "</body></html>"
                            ),
                            address.port()
                        );
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(), body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                        return;
                    }
                    let body = concat!(
                        "<html><head><title>OpenTopia browser test</title></head>",
                        "<body><h1>Browser runtime works</h1>",
                        "<button id='press' onclick=\"this.textContent='Pressed'\">Press</button>",
                        "<input id='field' /></body></html>"
                    );
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });

        let mut config = BrowserRuntimeConfig::default();
        let data_root =
            std::env::temp_dir().join(format!("opentopia-browser-test-{}", Uuid::new_v4()));
        config.data_root = data_root.clone();
        config.startup_timeout = Duration::from_secs(20);
        config.max_download_bytes = 1024;
        let runtime = LocalBrowserRuntime::new(config);
        let session = BrowserSessionId::new();
        let spec = BrowserSessionSpec {
            session_id: session,
            profile_id: BrowserProfileId::new("runtime-smoke").unwrap(),
            profile_persistence: BrowserProfilePersistence::Ephemeral,
        };
        let ephemeral_root = data_root.join("ephemeral").join("runtime-smoke");
        let info = runtime.create_session(spec).await.unwrap();
        assert_eq!(
            info.profile_persistence,
            BrowserProfilePersistence::Ephemeral
        );
        let conflict = runtime
            .create_session(BrowserSessionSpec::from(session))
            .await
            .unwrap_err();
        assert!(matches!(
            conflict,
            BrowserError::SessionProfileConflict { session: conflicted } if conflicted == session
        ));
        let url = format!("http://{address}/");

        runtime
            .grant_network_access(
                session,
                BrowserNetworkGrant::new([address.ip().to_string()]).unwrap(),
            )
            .await
            .unwrap();

        runtime
            .navigate(session, BrowserNavigateRequest::new(url))
            .await
            .unwrap();
        let observation = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        assert!(observation.text.contains("Browser runtime works"));
        let press = observation
            .nodes
            .iter()
            .find(|node| node.name == "Press")
            .expect("press button must be observable");

        let screenshot = runtime.screenshot(session).await.unwrap();
        assert!(matches!(
            screenshot.contents.first(),
            Some(BrowserContent::Image { bytes, .. }) if bytes.starts_with(b"\x89PNG")
        ));

        let click_receipt = runtime
            .perform(
                session,
                observation.observation_id,
                press.node_ref,
                BrowserAction::Click,
            )
            .await
            .unwrap();
        assert!(click_receipt.verification.page_changed);
        assert!(click_receipt.verification.text_changed);

        assert!(matches!(
            runtime
                .download(
                    session,
                    BrowserDownloadRequest {
                        url: format!("http://{address}/large-download"),
                        expected_filename: Some("large.bin".to_string()),
                        timeout: Some(Duration::from_secs(5)),
                    },
                )
                .await,
            Err(BrowserError::DownloadTooLarge { maximum: 1024 })
        ));

        assert!(matches!(
            runtime
                .perform(
                    session,
                    observation.observation_id,
                    press.node_ref,
                    BrowserAction::Click,
                )
                .await,
            Err(BrowserError::StaleObservation { .. })
        ));

        let refreshed = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        let field = refreshed
            .nodes
            .iter()
            .find(|node| node.tag_name == "input")
            .expect("input must be observable");
        runtime
            .perform(
                session,
                refreshed.observation_id,
                field.node_ref,
                BrowserAction::Type {
                    text: "OpenTopia".to_string(),
                    clear_first: true,
                },
            )
            .await
            .unwrap();

        assert!(matches!(
            runtime
                .navigate(
                    session,
                    BrowserNavigateRequest::new(format!("http://{address}/redirect")),
                )
                .await,
            Err(BrowserError::NetworkBlocked { ref host }) if host == "localhost"
        ));

        runtime
            .grant_network_access(session, BrowserNetworkGrant::new(["localhost"]).unwrap())
            .await
            .unwrap();
        runtime
            .navigate(
                session,
                BrowserNavigateRequest::new(format!("http://{address}/complex")),
            )
            .await
            .unwrap();
        let complex = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        assert!(complex.frames.len() >= 3);
        assert!(!complex.accessibility_tree.is_empty());
        assert!(complex
            .nodes
            .iter()
            .any(|node| node.name == "Nested shadow action"));
        assert!(
            complex
                .nodes
                .iter()
                .any(|node| node.name == "Frame shadow action"),
            "captured frames: {:?}; nodes: {:?}",
            complex.frames,
            complex
                .nodes
                .iter()
                .map(|node| (&node.name, &node.frame_ref))
                .collect::<Vec<_>>()
        );
        let initial_target = complex
            .targets
            .iter()
            .find(|target| target.active)
            .unwrap()
            .target_ref
            .clone();
        let select = complex
            .nodes
            .iter()
            .find(|node| node.tag_name == "select")
            .unwrap();
        runtime
            .perform(
                session,
                complex.observation_id,
                select.node_ref,
                BrowserAction::Select {
                    value: "pro".to_string(),
                },
            )
            .await
            .unwrap();
        let complex = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        assert!(complex.text.contains("selected:pro"));
        let popup = complex
            .nodes
            .iter()
            .find(|node| node.name == "Open popup")
            .unwrap();
        runtime
            .perform(
                session,
                complex.observation_id,
                popup.node_ref,
                BrowserAction::Click,
            )
            .await
            .unwrap();
        let popup_observation = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        assert!(popup_observation.targets.len() >= 2);
        assert!(popup_observation.text.contains("Popup ready"));
        runtime
            .switch_target(session, initial_target)
            .await
            .unwrap();
        let complex = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        let dialog = complex
            .nodes
            .iter()
            .find(|node| node.name == "Show dialog")
            .unwrap();
        runtime
            .perform(
                session,
                complex.observation_id,
                dialog.node_ref,
                BrowserAction::Click,
            )
            .await
            .unwrap();
        let after_dialog = runtime
            .observe(session, BrowserObserveOptions::default())
            .await
            .unwrap();
        assert!(after_dialog
            .dialogs
            .iter()
            .any(|dialog| { dialog.message == "fixture dialog" && dialog.handled }));

        runtime.close_session(session).await.unwrap();
        assert!(!ephemeral_root.exists());
        server.abort();
    }
}
