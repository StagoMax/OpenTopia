//! A capability-scoped desktop computer runtime.
//!
//! The runtime deliberately exposes windows rather than the whole desktop. A model must first
//! observe a user-approved window and every input action carries the resulting observation ID.
//! The Windows implementation is intentionally conservative: it fails closed when focus, the
//! process identity, or the capture bounds change.

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

pub const MAX_COMPUTER_WINDOWS: usize = 128;
pub const MAX_COMPUTER_SCREENSHOT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_COMPUTER_IMAGE_EDGE: u32 = 1_440;
const OBSERVATION_TTL_SECONDS: i64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComputerSessionId(Uuid);

impl ComputerSessionId {
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

impl Default for ComputerSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ComputerSessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenRect {
    pub fn contains_image_point(self, x: u32, y: u32, image_width: u32, image_height: u32) -> bool {
        image_width > 0 && image_height > 0 && x < image_width && y < image_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowTarget {
    /// Opaque, runtime-issued identifier. On Windows this is formatted from an HWND but callers
    /// must never synthesize it: `observe` verifies it against a live process before binding it.
    pub window_id: String,
    pub process_id: u32,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    pub bounds: ScreenRect,
    pub is_foreground: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveOptions {
    #[serde(default = "default_true")]
    pub include_screenshot: bool,
    #[serde(default)]
    pub include_accessibility_tree: bool,
}

impl Default for ObserveOptions {
    fn default() -> Self {
        Self {
            include_screenshot: true,
            include_accessibility_tree: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerObservation {
    pub observation_id: String,
    pub session_id: ComputerSessionId,
    pub target: WindowTarget,
    /// Coordinates in computer actions are relative to this image, never to CSS/DIP coordinates.
    pub capture_rect: ScreenRect,
    pub image_width: u32,
    pub image_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ComputerScreenshot>,
    /// Reserved for a future UIA adapter. It is omitted rather than fabricating accessibility data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_tree: Option<Value>,
    pub unstable: bool,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerScreenshot {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerMouseButton {
    Left,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComputerAction {
    Click {
        observation_id: String,
        x: u32,
        y: u32,
        #[serde(default)]
        button: ComputerMouseButton,
    },
    Type {
        observation_id: String,
        text: String,
    },
    Keypress {
        observation_id: String,
        key: String,
    },
    Scroll {
        observation_id: String,
        delta_y: i32,
    },
    Drag {
        observation_id: String,
        start_x: u32,
        start_y: u32,
        end_x: u32,
        end_y: u32,
    },
    Wait {
        observation_id: String,
        duration_ms: u64,
    },
}

impl ComputerAction {
    pub fn observation_id(&self) -> &str {
        match self {
            Self::Click { observation_id, .. }
            | Self::Type { observation_id, .. }
            | Self::Keypress { observation_id, .. }
            | Self::Scroll { observation_id, .. }
            | Self::Drag { observation_id, .. }
            | Self::Wait { observation_id, .. } => observation_id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Click { .. } => "click",
            Self::Type { .. } => "type",
            Self::Keypress { .. } => "keypress",
            Self::Scroll { .. } => "scroll",
            Self::Drag { .. } => "drag",
            Self::Wait { .. } => "wait",
        }
    }

    pub fn contains_sensitive_text(&self) -> bool {
        match self {
            Self::Type { text, .. } => looks_sensitive_text(text),
            _ => false,
        }
    }
}

impl Default for ComputerMouseButton {
    fn default() -> Self {
        Self::Left
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerActionReceipt {
    pub session_id: ComputerSessionId,
    pub observation_id: String,
    pub target: WindowTarget,
    pub action: String,
    pub sequence: u64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_redacted: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct ComputerPolicyContext {
    pub session_id: ComputerSessionId,
    pub thread_id: Option<Uuid>,
}

#[derive(Debug, Error)]
pub enum ComputerError {
    #[error("Computer Use is unavailable on this operating system")]
    UnsupportedPlatform,
    #[error("The requested desktop window is no longer available")]
    WindowNotFound,
    #[error(
        "The requested desktop window belongs to a different process than the observed window"
    )]
    TargetIdentityChanged,
    #[error("The requested action requires a current observation")]
    ObservationNotFound,
    #[error("The observation is stale; observe the window again before acting")]
    StaleObservation,
    #[error("The target window could not be made foreground; no input was injected")]
    ForegroundDenied,
    #[error("The target is a protected system or credential window")]
    ProtectedWindow,
    #[error("Input text appears to contain a password, token, or other secret and is refused")]
    SensitiveInputRefused,
    #[error("Unsupported computer key: {0}")]
    UnsupportedKey(String),
    #[error("Computer action coordinate is outside the observed screenshot")]
    InvalidCoordinate,
    #[error("Computer screenshot is {actual} bytes, exceeding the {maximum}-byte limit")]
    ScreenshotTooLarge { actual: usize, maximum: usize },
    #[error("Computer operation failed: {0}")]
    Platform(String),
}

#[async_trait]
pub trait ComputerRuntime: Send + Sync {
    async fn list_windows(
        &self,
        session: ComputerSessionId,
    ) -> Result<Vec<WindowTarget>, ComputerError>;

    async fn observe(
        &self,
        session: ComputerSessionId,
        target: WindowTarget,
        options: ObserveOptions,
    ) -> Result<ComputerObservation, ComputerError>;

    /// Resolve the observation's bound target for policy inspection without exposing input
    /// capability. The returned target is valid only as context for the immediately following
    /// action; `perform` revalidates it again.
    async fn target_for_observation(
        &self,
        session: ComputerSessionId,
        observation_id: &str,
    ) -> Result<WindowTarget, ComputerError>;

    async fn perform(
        &self,
        session: ComputerSessionId,
        action: ComputerAction,
    ) -> Result<ComputerActionReceipt, ComputerError>;

    async fn close_session(&self, session: ComputerSessionId) -> Result<(), ComputerError>;
}

#[derive(Debug, Clone)]
pub struct ComputerRuntimeConfig {
    pub max_windows: usize,
    pub max_image_edge: u32,
    pub max_screenshot_bytes: usize,
    pub observation_ttl_seconds: i64,
}

impl Default for ComputerRuntimeConfig {
    fn default() -> Self {
        Self {
            max_windows: MAX_COMPUTER_WINDOWS,
            max_image_edge: MAX_COMPUTER_IMAGE_EDGE,
            max_screenshot_bytes: MAX_COMPUTER_SCREENSHOT_BYTES,
            observation_ttl_seconds: OBSERVATION_TTL_SECONDS,
        }
    }
}

#[derive(Clone)]
pub struct LocalComputerRuntime {
    config: Arc<ComputerRuntimeConfig>,
    sessions: Arc<Mutex<HashMap<ComputerSessionId, ComputerSessionState>>>,
}

#[derive(Default)]
struct ComputerSessionState {
    windows: HashMap<String, WindowTarget>,
    observations: HashMap<String, ObservationBinding>,
    next_sequence: u64,
}

#[derive(Clone)]
struct ObservationBinding {
    target: WindowTarget,
    capture_rect: ScreenRect,
    image_width: u32,
    image_height: u32,
    captured_at: DateTime<Utc>,
}

impl LocalComputerRuntime {
    pub fn new(config: ComputerRuntimeConfig) -> Self {
        Self {
            config: Arc::new(config),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn observation(
        &self,
        session: ComputerSessionId,
        observation_id: &str,
    ) -> Result<(ObservationBinding, u64), ComputerError> {
        let mut sessions = self.sessions.lock().await;
        let state = sessions.entry(session).or_default();
        let observation = state
            .observations
            .get(observation_id)
            .cloned()
            .ok_or(ComputerError::ObservationNotFound)?;
        if Utc::now() - observation.captured_at
            > ChronoDuration::seconds(self.config.observation_ttl_seconds)
        {
            state.observations.remove(observation_id);
            return Err(ComputerError::StaleObservation);
        }
        state.next_sequence = state.next_sequence.saturating_add(1);
        Ok((observation, state.next_sequence))
    }
}

impl Default for LocalComputerRuntime {
    fn default() -> Self {
        Self::new(ComputerRuntimeConfig::default())
    }
}

#[async_trait]
impl ComputerRuntime for LocalComputerRuntime {
    async fn list_windows(
        &self,
        session: ComputerSessionId,
    ) -> Result<Vec<WindowTarget>, ComputerError> {
        let max_windows = self.config.max_windows;
        let windows = tokio::task::spawn_blocking(move || platform_list_windows(max_windows))
            .await
            .map_err(|error| ComputerError::Platform(error.to_string()))??;
        let mut sessions = self.sessions.lock().await;
        let state = sessions.entry(session).or_default();
        state
            .windows
            .retain(|window_id, _| windows.iter().any(|window| &window.window_id == window_id));
        Ok(windows)
    }

    async fn observe(
        &self,
        session: ComputerSessionId,
        target: WindowTarget,
        options: ObserveOptions,
    ) -> Result<ComputerObservation, ComputerError> {
        let config = self.config.clone();
        let current = tokio::task::spawn_blocking({
            let target = target.clone();
            move || platform_get_window(&target.window_id)
        })
        .await
        .map_err(|error| ComputerError::Platform(error.to_string()))??;
        ensure_same_window(&target, &current)?;
        ensure_controllable_window(&current)?;

        let screenshot = if options.include_screenshot {
            let target = current.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    platform_capture_window(
                        &target,
                        config.max_image_edge,
                        config.max_screenshot_bytes,
                    )
                })
                .await
                .map_err(|error| ComputerError::Platform(error.to_string()))??,
            )
        } else {
            None
        };
        let (image_width, image_height) = screenshot
            .as_ref()
            .map(|image| (image.width, image.height))
            .unwrap_or((current.bounds.width, current.bounds.height));
        let observation_id = format!("obs_{}", Uuid::new_v4().simple());
        let captured_at = Utc::now();
        let binding = ObservationBinding {
            target: current.clone(),
            capture_rect: current.bounds,
            image_width,
            image_height,
            captured_at,
        };
        let mut sessions = self.sessions.lock().await;
        let state = sessions.entry(session).or_default();
        state
            .windows
            .insert(current.window_id.clone(), current.clone());
        state.observations.insert(observation_id.clone(), binding);
        // Observations are capabilities, not a history store. Keeping only current frames limits
        // memory and makes stale-action failures deterministic.
        state.observations.retain(|_, item| {
            captured_at - item.captured_at
                <= ChronoDuration::seconds(self.config.observation_ttl_seconds)
        });
        Ok(ComputerObservation {
            observation_id,
            session_id: session,
            capture_rect: current.bounds,
            target: current,
            image_width,
            image_height,
            screenshot: screenshot.map(|image| ComputerScreenshot {
                mime_type: "image/png".to_string(),
                bytes: image.png,
            }),
            accessibility_tree: None,
            unstable: false,
            captured_at,
        })
    }

    async fn perform(
        &self,
        session: ComputerSessionId,
        action: ComputerAction,
    ) -> Result<ComputerActionReceipt, ComputerError> {
        if action.contains_sensitive_text() {
            return Err(ComputerError::SensitiveInputRefused);
        }
        let (observation, sequence) = self.observation(session, action.observation_id()).await?;
        let current = tokio::task::spawn_blocking({
            let window_id = observation.target.window_id.clone();
            move || platform_get_window(&window_id)
        })
        .await
        .map_err(|error| ComputerError::Platform(error.to_string()))??;
        ensure_same_window(&observation.target, &current)?;
        ensure_controllable_window(&current)?;
        if current.bounds != observation.capture_rect {
            return Err(ComputerError::StaleObservation);
        }

        let target = current.clone();
        let action_for_platform = action.clone();
        tokio::task::spawn_blocking(move || {
            platform_perform_action(&target, &observation, &action_for_platform)
        })
        .await
        .map_err(|error| ComputerError::Platform(error.to_string()))??;

        Ok(ComputerActionReceipt {
            session_id: session,
            observation_id: action.observation_id().to_string(),
            target: current,
            action: action.kind().to_string(),
            sequence,
            status: "executed".to_string(),
            input_redacted: matches!(action, ComputerAction::Type { .. }).then_some(true),
        })
    }

    async fn target_for_observation(
        &self,
        session: ComputerSessionId,
        observation_id: &str,
    ) -> Result<WindowTarget, ComputerError> {
        let sessions = self.sessions.lock().await;
        let state = sessions
            .get(&session)
            .ok_or(ComputerError::ObservationNotFound)?;
        let observation = state
            .observations
            .get(observation_id)
            .ok_or(ComputerError::ObservationNotFound)?;
        if Utc::now() - observation.captured_at
            > ChronoDuration::seconds(self.config.observation_ttl_seconds)
        {
            return Err(ComputerError::StaleObservation);
        }
        Ok(observation.target.clone())
    }

    async fn close_session(&self, session: ComputerSessionId) -> Result<(), ComputerError> {
        self.sessions.lock().await.remove(&session);
        Ok(())
    }
}

fn default_true() -> bool {
    true
}

fn ensure_same_window(
    expected: &WindowTarget,
    current: &WindowTarget,
) -> Result<(), ComputerError> {
    if expected.window_id != current.window_id || expected.process_id != current.process_id {
        return Err(ComputerError::TargetIdentityChanged);
    }
    Ok(())
}

fn ensure_controllable_window(target: &WindowTarget) -> Result<(), ComputerError> {
    let executable = target
        .executable
        .as_deref()
        .unwrap_or_default()
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(
        executable.as_str(),
        "consent.exe" | "logonui.exe" | "credentialuibroker.exe" | "lockapp.exe"
    ) {
        return Err(ComputerError::ProtectedWindow);
    }
    Ok(())
}

fn looks_sensitive_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("password")
        || lower.contains("passwd")
        || lower.contains("api_key")
        || lower.contains("api-key")
        || lower.contains("secret")
        || lower.starts_with("sk-")
        || lower.starts_with("akia")
        || text.split('.').count() == 3 && text.len() > 48
}

struct CapturedWindow {
    png: Vec<u8>,
    width: u32,
    height: u32,
}

#[cfg(not(windows))]
fn platform_list_windows(_max_windows: usize) -> Result<Vec<WindowTarget>, ComputerError> {
    Err(ComputerError::UnsupportedPlatform)
}

#[cfg(not(windows))]
fn platform_get_window(_window_id: &str) -> Result<WindowTarget, ComputerError> {
    Err(ComputerError::UnsupportedPlatform)
}

#[cfg(not(windows))]
fn platform_capture_window(
    _target: &WindowTarget,
    _max_image_edge: u32,
    _max_screenshot_bytes: usize,
) -> Result<CapturedWindow, ComputerError> {
    Err(ComputerError::UnsupportedPlatform)
}

#[cfg(not(windows))]
fn platform_perform_action(
    _target: &WindowTarget,
    _observation: &ObservationBinding,
    _action: &ComputerAction,
) -> Result<(), ComputerError> {
    Err(ComputerError::UnsupportedPlatform)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use png::{BitDepth, ColorType, Encoder};
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
        GetWindowDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT,
        DIB_RGB_COLORS, SRCCOPY,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
        MOUSEEVENTF_WHEEL, MOUSEINPUT, VK_BACK, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_RETURN, VK_RIGHT,
        VK_TAB, VK_UP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowTextLengthW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SetForegroundWindow, ShowWindow, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN, SW_RESTORE,
    };

    pub(super) fn list_windows(max_windows: usize) -> Result<Vec<WindowTarget>, ComputerError> {
        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> i32 {
            let windows = &mut *(lparam as *mut Vec<WindowTarget>);
            if let Some(window) = read_window(hwnd) {
                windows.push(window);
            }
            1
        }

        let mut windows: Vec<WindowTarget> = Vec::new();
        unsafe {
            EnumWindows(
                Some(callback),
                &mut windows as *mut Vec<WindowTarget> as LPARAM,
            );
        }
        windows.sort_by(|left, right| {
            right
                .is_foreground
                .cmp(&left.is_foreground)
                .then_with(|| left.title.cmp(&right.title))
        });
        windows.truncate(max_windows);
        Ok(windows)
    }

    pub(super) fn get_window(window_id: &str) -> Result<WindowTarget, ComputerError> {
        let hwnd = parse_window_id(window_id)?;
        read_window(hwnd).ok_or(ComputerError::WindowNotFound)
    }

    fn parse_window_id(window_id: &str) -> Result<HWND, ComputerError> {
        let raw = window_id
            .strip_prefix("hwnd:")
            .ok_or(ComputerError::WindowNotFound)?;
        let value = usize::from_str_radix(raw, 16).map_err(|_| ComputerError::WindowNotFound)?;
        Ok(value as HWND)
    }

    fn read_window(hwnd: HWND) -> Option<WindowTarget> {
        unsafe {
            if hwnd.is_null()
                || IsWindow(hwnd) == 0
                || IsWindowVisible(hwnd) == 0
                || IsIconic(hwnd) != 0
            {
                return None;
            }
            let title_length = GetWindowTextLengthW(hwnd);
            if title_length <= 0 {
                return None;
            }
            let mut title = vec![0_u16; title_length as usize + 1];
            let read = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            if read <= 0 {
                return None;
            }
            let title = String::from_utf16_lossy(&title[..read as usize])
                .trim()
                .to_string();
            if title.is_empty() {
                return None;
            }
            let mut rect: RECT = zeroed();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return None;
            }
            let width = (rect.right - rect.left).max(0) as u32;
            let height = (rect.bottom - rect.top).max(0) as u32;
            if width == 0 || height == 0 {
                return None;
            }
            let mut process_id = 0_u32;
            GetWindowThreadProcessId(hwnd, &mut process_id);
            if process_id == 0 {
                return None;
            }
            let executable = process_image_path(process_id);
            Some(WindowTarget {
                window_id: format!("hwnd:{:016X}", hwnd as usize),
                process_id,
                title: truncate_title(title),
                executable,
                bounds: ScreenRect {
                    x: rect.left,
                    y: rect.top,
                    width,
                    height,
                },
                is_foreground: GetForegroundWindow() == hwnd,
            })
        }
    }

    fn truncate_title(mut title: String) -> String {
        const MAX_TITLE_CHARS: usize = 160;
        if title.chars().count() > MAX_TITLE_CHARS {
            title = title.chars().take(MAX_TITLE_CHARS).collect::<String>();
            title.push_str("...");
        }
        title
    }

    unsafe fn process_image_path(process_id: u32) -> Option<String> {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if handle.is_null() {
            return None;
        }
        let mut output = vec![0_u16; 32_768];
        let mut length = output.len() as u32;
        let success = QueryFullProcessImageNameW(handle, 0, output.as_mut_ptr(), &mut length);
        CloseHandle(handle);
        (success != 0 && length > 0).then(|| String::from_utf16_lossy(&output[..length as usize]))
    }

    pub(super) fn capture_window(
        target: &WindowTarget,
        max_image_edge: u32,
        max_screenshot_bytes: usize,
    ) -> Result<CapturedWindow, ComputerError> {
        let hwnd = parse_window_id(&target.window_id)?;
        let width = target.bounds.width;
        let height = target.bounds.height;
        if width == 0 || height == 0 {
            return Err(ComputerError::WindowNotFound);
        }
        let mut pixels = unsafe { capture_rgba(hwnd, width, height)? };
        let (image_width, image_height) =
            resize_to_limit(&mut pixels, width, height, max_image_edge);
        let png = encode_png(&pixels, image_width, image_height)?;
        if png.len() > max_screenshot_bytes {
            return Err(ComputerError::ScreenshotTooLarge {
                actual: png.len(),
                maximum: max_screenshot_bytes,
            });
        }
        Ok(CapturedWindow {
            png,
            width: image_width,
            height: image_height,
        })
    }

    unsafe fn capture_rgba(hwnd: HWND, width: u32, height: u32) -> Result<Vec<u8>, ComputerError> {
        let source = GetWindowDC(hwnd);
        if source.is_null() {
            return Err(ComputerError::Platform("GetWindowDC failed".to_string()));
        }
        let memory = CreateCompatibleDC(source);
        if memory.is_null() {
            ReleaseDC(hwnd, source);
            return Err(ComputerError::Platform(
                "CreateCompatibleDC failed".to_string(),
            ));
        }
        let bitmap = CreateCompatibleBitmap(source, width as i32, height as i32);
        if bitmap.is_null() {
            DeleteDC(memory);
            ReleaseDC(hwnd, source);
            return Err(ComputerError::Platform(
                "CreateCompatibleBitmap failed".to_string(),
            ));
        }
        let old = SelectObject(memory, bitmap as *mut c_void);
        let copied = BitBlt(
            memory,
            0,
            0,
            width as i32,
            height as i32,
            source,
            0,
            0,
            SRCCOPY | CAPTUREBLT,
        );
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..zeroed()
        };
        let mut bgra = vec![0_u8; width as usize * height as usize * 4];
        let copied_pixels = if copied != 0 {
            GetDIBits(
                memory,
                bitmap,
                0,
                height,
                bgra.as_mut_ptr() as *mut c_void,
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };
        SelectObject(memory, old);
        DeleteObject(bitmap as *mut c_void);
        DeleteDC(memory);
        ReleaseDC(hwnd, source);
        if copied_pixels == 0 {
            return Err(ComputerError::Platform("window capture failed".to_string()));
        }
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(bgra)
    }

    fn resize_to_limit(pixels: &mut Vec<u8>, width: u32, height: u32, max_edge: u32) -> (u32, u32) {
        let longest = width.max(height);
        if longest <= max_edge || max_edge == 0 {
            return (width, height);
        }
        let scaled_width = ((width as u64 * max_edge as u64) / longest as u64).max(1) as u32;
        let scaled_height = ((height as u64 * max_edge as u64) / longest as u64).max(1) as u32;
        let source = std::mem::take(pixels);
        let mut output = vec![0_u8; scaled_width as usize * scaled_height as usize * 4];
        for y in 0..scaled_height {
            for x in 0..scaled_width {
                let source_x = (x as u64 * width as u64 / scaled_width as u64) as usize;
                let source_y = (y as u64 * height as u64 / scaled_height as u64) as usize;
                let src = (source_y * width as usize + source_x) * 4;
                let dst = (y as usize * scaled_width as usize + x as usize) * 4;
                output[dst..dst + 4].copy_from_slice(&source[src..src + 4]);
            }
        }
        *pixels = output;
        (scaled_width, scaled_height)
    }

    fn encode_png(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ComputerError> {
        let mut output = Vec::new();
        let mut encoder = Encoder::new(&mut output, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| ComputerError::Platform(error.to_string()))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| ComputerError::Platform(error.to_string()))?;
        drop(writer);
        Ok(output)
    }

    pub(super) fn perform_action(
        target: &WindowTarget,
        observation: &ObservationBinding,
        action: &ComputerAction,
    ) -> Result<(), ComputerError> {
        let hwnd = parse_window_id(&target.window_id)?;
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
            if GetForegroundWindow() != hwnd {
                return Err(ComputerError::ForegroundDenied);
            }
        }
        match action {
            ComputerAction::Click { x, y, button, .. } => {
                let (screen_x, screen_y) = image_to_screen(observation, *x, *y)?;
                move_mouse(screen_x, screen_y)?;
                match button {
                    ComputerMouseButton::Left => send_mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0)?,
                    ComputerMouseButton::Right => send_mouse(MOUSEEVENTF_RIGHTDOWN, 0, 0, 0)?,
                }
                match button {
                    ComputerMouseButton::Left => send_mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0),
                    ComputerMouseButton::Right => send_mouse(MOUSEEVENTF_RIGHTUP, 0, 0, 0),
                }
            }
            ComputerAction::Type { text, .. } => send_unicode_text(text),
            ComputerAction::Keypress { key, .. } => send_key(key),
            ComputerAction::Scroll { delta_y, .. } => {
                let delta = (*delta_y).clamp(-12_000, 12_000);
                send_mouse(MOUSEEVENTF_WHEEL, 0, 0, delta as u32)
            }
            ComputerAction::Drag {
                start_x,
                start_y,
                end_x,
                end_y,
                ..
            } => {
                let (start_screen_x, start_screen_y) =
                    image_to_screen(observation, *start_x, *start_y)?;
                let (end_screen_x, end_screen_y) = image_to_screen(observation, *end_x, *end_y)?;
                move_mouse(start_screen_x, start_screen_y)?;
                send_mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0)?;
                move_mouse(end_screen_x, end_screen_y)?;
                send_mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0)
            }
            ComputerAction::Wait { duration_ms, .. } => {
                std::thread::sleep(std::time::Duration::from_millis(
                    (*duration_ms).clamp(1, 30_000),
                ));
                Ok(())
            }
        }
    }

    fn image_to_screen(
        observation: &ObservationBinding,
        x: u32,
        y: u32,
    ) -> Result<(i32, i32), ComputerError> {
        if !observation.capture_rect.contains_image_point(
            x,
            y,
            observation.image_width,
            observation.image_height,
        ) {
            return Err(ComputerError::InvalidCoordinate);
        }
        let screen_x = observation.capture_rect.x
            + ((x as u64 * observation.capture_rect.width as u64) / observation.image_width as u64)
                as i32;
        let screen_y = observation.capture_rect.y
            + ((y as u64 * observation.capture_rect.height as u64)
                / observation.image_height as u64) as i32;
        Ok((screen_x, screen_y))
    }

    fn move_mouse(screen_x: i32, screen_y: i32) -> Result<(), ComputerError> {
        unsafe {
            let virtual_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let virtual_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let virtual_width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let virtual_height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if virtual_width <= 1 || virtual_height <= 1 {
                return Err(ComputerError::Platform(
                    "invalid virtual screen bounds".to_string(),
                ));
            }
            let x = (((screen_x - virtual_x) as i64 * 65_535) / (virtual_width as i64 - 1))
                .clamp(0, 65_535) as i32;
            let y = (((screen_y - virtual_y) as i64 * 65_535) / (virtual_height as i64 - 1))
                .clamp(0, 65_535) as i32;
            send_mouse(
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                x,
                y,
                0,
            )
        }
    }

    fn send_mouse(flags: u32, dx: i32, dy: i32, mouse_data: u32) -> Result<(), ComputerError> {
        unsafe {
            let mut input: INPUT = zeroed();
            input.r#type = INPUT_MOUSE;
            input.Anonymous = INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: mouse_data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            };
            if SendInput(1, &input, size_of::<INPUT>() as i32) != 1 {
                return Err(ComputerError::Platform(
                    "SendInput mouse injection failed".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn send_unicode_text(text: &str) -> Result<(), ComputerError> {
        for code_unit in text.encode_utf16() {
            send_keyboard(0, code_unit, KEYEVENTF_UNICODE)?;
            send_keyboard(0, code_unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)?;
        }
        Ok(())
    }

    fn send_key(key: &str) -> Result<(), ComputerError> {
        let code = match key.trim().to_ascii_uppercase().as_str() {
            "ENTER" => VK_RETURN,
            "TAB" => VK_TAB,
            "ESC" | "ESCAPE" => VK_ESCAPE,
            "BACKSPACE" => VK_BACK,
            "LEFT" | "ARROWLEFT" => VK_LEFT,
            "RIGHT" | "ARROWRIGHT" => VK_RIGHT,
            "UP" | "ARROWUP" => VK_UP,
            "DOWN" | "ARROWDOWN" => VK_DOWN,
            other => return Err(ComputerError::UnsupportedKey(other.to_string())),
        };
        send_keyboard(code as u16, 0, 0)?;
        send_keyboard(code as u16, 0, KEYEVENTF_KEYUP)
    }

    fn send_keyboard(virtual_key: u16, unicode: u16, flags: u32) -> Result<(), ComputerError> {
        unsafe {
            let mut input: INPUT = zeroed();
            input.r#type = INPUT_KEYBOARD;
            input.Anonymous = INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: virtual_key,
                    wScan: unicode,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            };
            if SendInput(1, &input, size_of::<INPUT>() as i32) != 1 {
                return Err(ComputerError::Platform(
                    "SendInput keyboard injection failed".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
fn platform_list_windows(max_windows: usize) -> Result<Vec<WindowTarget>, ComputerError> {
    windows::list_windows(max_windows)
}

#[cfg(windows)]
fn platform_get_window(window_id: &str) -> Result<WindowTarget, ComputerError> {
    windows::get_window(window_id)
}

#[cfg(windows)]
fn platform_capture_window(
    target: &WindowTarget,
    max_image_edge: u32,
    max_screenshot_bytes: usize,
) -> Result<CapturedWindow, ComputerError> {
    windows::capture_window(target, max_image_edge, max_screenshot_bytes)
}

#[cfg(windows)]
fn platform_perform_action(
    target: &WindowTarget,
    observation: &ObservationBinding,
    action: &ComputerAction,
) -> Result<(), ComputerError> {
    windows::perform_action(target, observation, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_input_is_never_forwarded_to_the_runtime() {
        assert!(ComputerAction::Type {
            observation_id: "obs".to_string(),
            text: "sk-test-secret-value".to_string(),
        }
        .contains_sensitive_text());
        assert!(!ComputerAction::Type {
            observation_id: "obs".to_string(),
            text: "hello from OpenTopia".to_string(),
        }
        .contains_sensitive_text());
    }

    #[test]
    fn screen_rect_rejects_out_of_frame_coordinates() {
        let rect = ScreenRect {
            x: -100,
            y: 20,
            width: 800,
            height: 600,
        };
        assert!(rect.contains_image_point(799, 599, 800, 600));
        assert!(!rect.contains_image_point(800, 599, 800, 600));
    }
}
