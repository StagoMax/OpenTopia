//! A bounded, session-isolated Chromium runtime.
//!
//! This module intentionally stops at the browser boundary: callers decide whether a URL or an
//! interaction needs approval, while this runtime owns the browser process and its per-session
//! profile. The `BrowserContent` enum is a richer result contract than the current text-only tool
//! result and can be adapted to a future multimodal message protocol without re-reading data.

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async, tungstenite::Message as WebSocketMessage, MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
const DEFAULT_MAX_SCREENSHOT_BYTES: usize = 8 * 1024 * 1024;

/// An opaque ID that should normally be derived from a thread ID. A session gets its own browser
/// process, user-data directory, cookie jar, cache, and download directory.
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

/// Configuration for a local Chrome or Edge process.
#[derive(Debug, Clone)]
pub struct BrowserRuntimeConfig {
    /// Use a specific Chrome/Edge executable. When omitted, Chrome and Edge are discovered from
    /// standard platform locations and `PATH` when a session is first used.
    pub executable: Option<PathBuf>,
    /// Browser state is stored below this directory in a directory named after the session ID.
    pub data_root: PathBuf,
    pub headless: bool,
    pub startup_timeout: Duration,
    pub command_timeout: Duration,
    pub max_snapshot_bytes: usize,
    pub max_screenshot_bytes: usize,
    /// Navigation and direct-download URLs are restricted to these schemes. Domain approval is
    /// deliberately left to the caller's policy layer.
    pub allowed_schemes: Vec<String>,
    /// Preserve browser profiles and downloads after `close_session`. Defaults to false so thread
    /// cookies and downloaded files do not become an unbounded local data store.
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
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            retain_session_data: false,
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
pub struct BrowserSnapshot {
    pub url: String,
    pub title: String,
    pub text: String,
    pub text_truncated: bool,
    /// A compact, model-oriented description of interactive elements and their CSS selectors.
    pub interactive_elements: Value,
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
    #[error("Invalid browser URL: {0}")]
    InvalidUrl(String),
    #[error("URL scheme is not allowed by this browser runtime: {0}")]
    DisallowedScheme(String),
    #[error("Invalid CSS selector: {0}")]
    InvalidSelector(String),
    #[error("Browser startup timed out after {0:?}")]
    StartupTimeout(Duration),
    #[error("Browser operation timed out while waiting for {0}")]
    Timeout(String),
    #[error("Browser protocol error: {0}")]
    Protocol(String),
    #[error("Browser command {method} failed: {message}")]
    Cdp { method: String, message: String },
    #[error("Screenshot is {actual} bytes, exceeding the configured {maximum}-byte limit")]
    ScreenshotTooLarge { actual: usize, maximum: usize },
    #[error("Download did not complete before the timeout")]
    DownloadTimeout,
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
    async fn navigate(
        &self,
        session: BrowserSessionId,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError>;
    async fn snapshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError>;
    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError>;
    async fn click(
        &self,
        session: BrowserSessionId,
        selector: BrowserSelector,
    ) -> Result<BrowserOutput, BrowserError>;
    async fn type_text(
        &self,
        session: BrowserSessionId,
        request: BrowserTypeRequest,
    ) -> Result<BrowserOutput, BrowserError>;
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
}

impl LocalBrowserRuntime {
    pub fn new(config: BrowserRuntimeConfig) -> Self {
        Self {
            config: Arc::new(config),
            sessions: Arc::new(Mutex::new(HashMap::new())),
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
            return Ok(session.clone());
        }

        let session = Arc::new(Mutex::new(
            LocalBrowserSession::start(session_id, self.config.clone()).await?,
        ));
        sessions.insert(session_id, session.clone());
        Ok(session)
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

    async fn snapshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.snapshot().await
    }

    async fn screenshot(&self, session: BrowserSessionId) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.screenshot().await
    }

    async fn click(
        &self,
        session: BrowserSessionId,
        selector: BrowserSelector,
    ) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.click(&selector).await
    }

    async fn type_text(
        &self,
        session: BrowserSessionId,
        request: BrowserTypeRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let runtime = self.session(session).await?;
        let mut runtime = runtime.lock().await;
        runtime.type_text(request).await
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

        let (session_dir, retain_session_data) = {
            let mut session = session.lock().await;
            let session_dir = session.session_dir.clone();
            let retain_session_data = session.retain_session_data;
            session.shutdown().await?;
            (session_dir, retain_session_data)
        };
        if !retain_session_data && session_dir.exists() {
            tokio::fs::remove_dir_all(session_dir).await?;
        }
        Ok(())
    }
}

struct LocalBrowserSession {
    page: CdpPage,
    child: Child,
    session_dir: PathBuf,
    download_dir: PathBuf,
    command_timeout: Duration,
    max_snapshot_bytes: usize,
    max_screenshot_bytes: usize,
    retain_session_data: bool,
}

impl LocalBrowserSession {
    async fn start(
        id: BrowserSessionId,
        config: Arc<BrowserRuntimeConfig>,
    ) -> Result<Self, BrowserError> {
        let executable = discover_browser_executable(config.executable.as_deref())?;
        let session_dir = config.data_root.join(id.as_uuid().to_string());
        let profile_dir = session_dir.join("profile");
        let download_dir = session_dir.join("downloads");
        tokio::fs::create_dir_all(&profile_dir).await?;
        tokio::fs::create_dir_all(&download_dir).await?;

        let mut command = Command::new(executable);
        command
            .arg("--remote-debugging-address=127.0.0.1")
            .arg("--remote-debugging-port=0")
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
            command.arg("--headless=new");
        }
        command.arg("about:blank");
        let child = command.spawn()?;

        let result = async {
            let port = wait_for_devtools_port(&profile_dir, config.startup_timeout).await?;
            let websocket_url = wait_for_page_websocket_url(port, config.startup_timeout).await?;
            let mut page = CdpPage::connect(&websocket_url, config.command_timeout).await?;
            if page
                .command(
                    "Browser.setDownloadBehavior",
                    json!({
                        "behavior": "allow",
                        "downloadPath": download_dir,
                        "eventsEnabled": true,
                    }),
                )
                .await
                .is_err()
            {
                page.command(
                    "Page.setDownloadBehavior",
                    json!({"behavior":"allow", "downloadPath": download_dir}),
                )
                .await?;
            }
            Ok::<_, BrowserError>(page)
        }
        .await;

        match result {
            Ok(page) => Ok(Self {
                page,
                child,
                session_dir,
                download_dir,
                command_timeout: config.command_timeout,
                max_snapshot_bytes: config.max_snapshot_bytes,
                max_screenshot_bytes: config.max_screenshot_bytes,
                retain_session_data: config.retain_session_data,
            }),
            Err(error) => {
                let mut child = child;
                let _ = child.kill().await;
                if !config.retain_session_data && session_dir.exists() {
                    let _ = tokio::fs::remove_dir_all(&session_dir).await;
                }
                Err(error)
            }
        }
    }

    async fn navigate(
        &mut self,
        request: BrowserNavigateRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let result = self
            .page
            .command("Page.navigate", json!({ "url": request.url }))
            .await?;
        if let Some(error_text) = result.get("errorText").and_then(Value::as_str) {
            return Err(BrowserError::Cdp {
                method: "Page.navigate".to_string(),
                message: error_text.to_string(),
            });
        }
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

    async fn snapshot(&mut self) -> Result<BrowserOutput, BrowserError> {
        let url = self.current_url().await?;
        let title = self.current_title().await?;
        let text = self
            .evaluate_value("document.body ? document.body.innerText : ''")
            .await?;
        let text = text.as_str().unwrap_or_default().to_string();
        let (text, text_truncated) = truncate_utf8(&text, self.max_snapshot_bytes);
        let interactive_elements = self
            .evaluate_value(INTERACTIVE_SNAPSHOT_SCRIPT)
            .await
            .unwrap_or_else(|_| json!([]));
        let snapshot = BrowserSnapshot {
            url: url.clone(),
            title: title.clone(),
            text: text.clone(),
            text_truncated,
            interactive_elements: interactive_elements.clone(),
        };
        Ok(BrowserOutput {
            url: Some(url),
            contents: vec![
                BrowserContent::Text {
                    text,
                    truncated: text_truncated,
                },
                BrowserContent::Json {
                    value: serde_json::to_value(snapshot)?,
                },
            ],
            metadata: json!({ "action": "snapshot", "title": title }),
        })
    }

    async fn screenshot(&mut self) -> Result<BrowserOutput, BrowserError> {
        let result = self
            .page
            .command("Page.captureScreenshot", json!({ "format": "png" }))
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
        Ok(BrowserOutput {
            url: Some(self.current_url().await?),
            contents: vec![BrowserContent::Image {
                mime_type: "image/png".to_string(),
                bytes,
            }],
            metadata: json!({ "action": "screenshot" }),
        })
    }

    async fn click(&mut self, selector: &BrowserSelector) -> Result<BrowserOutput, BrowserError> {
        let node_id = self.find_node_id(selector).await?;
        let model = self
            .page
            .command("DOM.getBoxModel", json!({ "nodeId": node_id }))
            .await?;
        let points = model
            .pointer("/model/content")
            .and_then(Value::as_array)
            .ok_or_else(|| BrowserError::Cdp {
                method: "DOM.getBoxModel".to_string(),
                message: "Element has no clickable box model.".to_string(),
            })?;
        if points.len() < 6 {
            return Err(BrowserError::Cdp {
                method: "DOM.getBoxModel".to_string(),
                message: "Element has an incomplete box model.".to_string(),
            });
        }
        let x =
            (points[0].as_f64().unwrap_or_default() + points[4].as_f64().unwrap_or_default()) / 2.0;
        let y =
            (points[1].as_f64().unwrap_or_default() + points[5].as_f64().unwrap_or_default()) / 2.0;
        self.page
            .command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
        self.page
            .command(
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
            )
            .await?;
        self.page_output("click", json!({ "selector": selector.as_str() }))
            .await
    }

    async fn type_text(
        &mut self,
        request: BrowserTypeRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let node_id = self.find_node_id(&request.selector).await?;
        self.page
            .command("DOM.focus", json!({ "nodeId": node_id }))
            .await?;
        if request.clear_first {
            let selector = serde_json::to_string(request.selector.as_str())?;
            self.evaluate_value(&format!(
                "(() => {{ const element = document.querySelector({selector}); if (!element) throw new Error('Element no longer exists'); if ('value' in element) {{ element.value = ''; element.dispatchEvent(new Event('input', {{ bubbles: true }})); element.dispatchEvent(new Event('change', {{ bubbles: true }})); }} else if (element.isContentEditable) {{ element.textContent = ''; }} }})()"
            ))
            .await?;
        }
        self.page
            .command("Input.insertText", json!({ "text": request.text }))
            .await?;
        self.page_output(
            "type",
            json!({ "selector": request.selector.as_str(), "clearFirst": request.clear_first }),
        )
        .await
    }

    async fn wait(&mut self, request: BrowserWaitRequest) -> Result<BrowserOutput, BrowserError> {
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

    async fn download(
        &mut self,
        request: BrowserDownloadRequest,
    ) -> Result<BrowserOutput, BrowserError> {
        let before = list_downloads(&self.download_dir).await?;
        self.page
            .command("Page.navigate", json!({ "url": request.url }))
            .await?;
        let download = wait_for_download(
            &self.download_dir,
            &before,
            request.expected_filename.as_deref(),
            request.timeout.unwrap_or(self.command_timeout),
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

    async fn find_node_id(&mut self, selector: &BrowserSelector) -> Result<i64, BrowserError> {
        let document = self
            .page
            .command("DOM.getDocument", json!({ "depth": 0 }))
            .await?;
        let root_id = document
            .pointer("/root/nodeId")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                BrowserError::Protocol("DOM.getDocument returned no root node ID".to_string())
            })?;
        let result = self
            .page
            .command(
                "DOM.querySelector",
                json!({ "nodeId": root_id, "selector": selector.as_str() }),
            )
            .await?;
        let node_id = result.get("nodeId").and_then(Value::as_i64).unwrap_or(0);
        if node_id == 0 {
            return Err(BrowserError::InvalidSelector(format!(
                "No element matches `{}`.",
                selector.as_str()
            )));
        }
        Ok(node_id)
    }

    async fn evaluate_value(&mut self, expression: &str) -> Result<Value, BrowserError> {
        let result = self
            .page
            .command(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true }),
            )
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

    async fn shutdown(&mut self) -> Result<(), BrowserError> {
        let _ = self.page.command("Browser.close", json!({})).await;
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

impl Drop for LocalBrowserSession {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

type CdpSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct CdpPage {
    write: futures_util::stream::SplitSink<CdpSocket, WebSocketMessage>,
    read: futures_util::stream::SplitStream<CdpSocket>,
    next_id: u64,
    command_timeout: Duration,
}

impl CdpPage {
    async fn connect(websocket_url: &str, command_timeout: Duration) -> Result<Self, BrowserError> {
        let (socket, _) = tokio::time::timeout(command_timeout, connect_async(websocket_url))
            .await
            .map_err(|_| BrowserError::Timeout("connecting to the local browser".to_string()))?
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;
        let (write, read) = socket.split();
        Ok(Self {
            write,
            read,
            next_id: 0,
            command_timeout,
        })
    }

    async fn command(&mut self, method: &str, params: Value) -> Result<Value, BrowserError> {
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        let command = json!({ "id": id, "method": method, "params": params });
        self.write
            .send(WebSocketMessage::Text(command.to_string()))
            .await
            .map_err(|error| BrowserError::Protocol(error.to_string()))?;

        loop {
            let incoming = tokio::time::timeout(self.command_timeout, self.read.next())
                .await
                .map_err(|_| BrowserError::Timeout(method.to_string()))?
                .ok_or_else(|| {
                    BrowserError::Protocol("Browser closed the DevTools connection".to_string())
                })
                .and_then(|message| {
                    message.map_err(|error| BrowserError::Protocol(error.to_string()))
                })?;
            match incoming {
                WebSocketMessage::Text(text) => {
                    let response: Value = serde_json::from_str(&text)?;
                    if response.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(error) = response.get("error") {
                        return Err(BrowserError::Cdp {
                            method: method.to_string(),
                            message: error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("Unknown DevTools error")
                                .to_string(),
                        });
                    }
                    return Ok(response.get("result").cloned().unwrap_or(Value::Null));
                }
                WebSocketMessage::Ping(payload) => {
                    self.write
                        .send(WebSocketMessage::Pong(payload))
                        .await
                        .map_err(|error| BrowserError::Protocol(error.to_string()))?;
                }
                WebSocketMessage::Close(_) => {
                    return Err(BrowserError::Protocol(
                        "Browser closed the DevTools connection".to_string(),
                    ));
                }
                WebSocketMessage::Binary(_)
                | WebSocketMessage::Pong(_)
                | WebSocketMessage::Frame(_) => {}
            }
        }
    }
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

async fn wait_for_page_websocket_url(port: u16, timeout: Duration) -> Result<String, BrowserError> {
    let client = reqwest::Client::builder().no_proxy().build()?;
    let endpoint = format!("http://127.0.0.1:{port}/json/list");
    let create_endpoint = format!("http://127.0.0.1:{port}/json/new?about:blank");
    let started = tokio::time::Instant::now();
    let mut created_target = false;
    loop {
        if let Ok(response) = client.get(&endpoint).send().await {
            if let Ok(targets) = response.json::<Vec<Value>>().await {
                if !created_target {
                    created_target = true;
                    if let Ok(response) = client.put(&create_endpoint).send().await {
                        if let Ok(target) = response.json::<Value>().await {
                            if let Some(websocket_url) =
                                target.get("webSocketDebuggerUrl").and_then(Value::as_str)
                            {
                                return Ok(websocket_url.to_string());
                            }
                        }
                    }
                }
                let page_targets = targets
                    .iter()
                    .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
                    .collect::<Vec<_>>();
                let target = page_targets
                    .iter()
                    .copied()
                    .find(|target| target.get("url").and_then(Value::as_str) == Some("about:blank"))
                    .or_else(|| page_targets.first().copied());
                if let Some(websocket_url) = target
                    .and_then(|target| target.get("webSocketDebuggerUrl").and_then(Value::as_str))
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
    directory: &Path,
    before: &HashSet<PathBuf>,
    expected_filename: Option<&str>,
    timeout: Duration,
) -> Result<BrowserDownload, BrowserError> {
    let started = tokio::time::Instant::now();
    let mut last_candidate: Option<(PathBuf, u64)> = None;
    loop {
        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if before.contains(&path)
                || path.extension().and_then(|value| value.to_str()) == Some("crdownload")
                || path.extension().and_then(|value| value.to_str()) == Some("tmp")
            {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().to_string();
            if expected_filename.is_some_and(|expected| expected != filename) {
                continue;
            }
            let metadata = entry.metadata().await?;
            if !metadata.is_file() {
                continue;
            }
            let bytes = metadata.len();
            if last_candidate.as_ref() == Some(&(path.clone(), bytes)) {
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
            return Err(BrowserError::DownloadTimeout);
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
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
  const text = (element) => (element.innerText || element.value || element.getAttribute('aria-label') || '')
    .replace(/\s+/g, ' ').trim().slice(0, 240);
  const cssPath = (element) => {
    if (element.id) return `#${CSS.escape(element.id)}`;
    const parts = [];
    for (let node = element; node && node.nodeType === Node.ELEMENT_NODE && node !== document.body; node = node.parentElement) {
      let part = node.tagName.toLowerCase();
      const siblings = Array.from(node.parentElement?.children || []).filter((child) => child.tagName === node.tagName);
      if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
      parts.unshift(part);
    }
    return `body > ${parts.join(' > ')}`;
  };
  return Array.from(document.querySelectorAll('a, button, input, textarea, select, [role="button"], [contenteditable="true"]'))
    .filter((element) => !element.disabled && element.getClientRects().length)
    .slice(0, max)
    .map((element) => ({
      selector: cssPath(element), tag: element.tagName.toLowerCase(), role: element.getAttribute('role'),
      text: text(element), ariaLabel: element.getAttribute('aria-label'), placeholder: element.getAttribute('placeholder'),
      type: element.getAttribute('type'), href: element.href || null
    }));
})()
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn selector_rejects_empty_values() {
        assert!(BrowserSelector::new("  ").is_err());
        assert_eq!(
            BrowserSelector::new("button.submit").unwrap().as_str(),
            "button.submit"
        );
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
                    let _ = socket.read(&mut request).await;
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
        config.data_root =
            std::env::temp_dir().join(format!("opentopia-browser-test-{}", Uuid::new_v4()));
        config.startup_timeout = Duration::from_secs(20);
        let runtime = LocalBrowserRuntime::new(config);
        let session = BrowserSessionId::new();
        let url = format!("http://{address}/");

        runtime
            .navigate(session, BrowserNavigateRequest::new(url))
            .await
            .unwrap();
        let snapshot = runtime.snapshot(session).await.unwrap();
        let text = snapshot
            .contents
            .iter()
            .find_map(|content| match content {
                BrowserContent::Text { text, .. } => Some(text),
                _ => None,
            })
            .unwrap();
        assert!(text.contains("Browser runtime works"));

        let screenshot = runtime.screenshot(session).await.unwrap();
        assert!(matches!(
            screenshot.contents.first(),
            Some(BrowserContent::Image { bytes, .. }) if bytes.starts_with(b"\x89PNG")
        ));

        runtime
            .click(session, BrowserSelector::new("#press").unwrap())
            .await
            .unwrap();
        runtime
            .type_text(
                session,
                BrowserTypeRequest {
                    selector: BrowserSelector::new("#field").unwrap(),
                    text: "OpenTopia".to_string(),
                    clear_first: true,
                },
            )
            .await
            .unwrap();

        runtime.close_session(session).await.unwrap();
        server.abort();
    }
}
