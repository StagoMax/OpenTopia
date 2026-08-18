//! Managed Chromium process lifecycle and profile storage.

use super::cdp_transport::CdpPage;
use super::{
    BrowserError, BrowserProfilePersistence, BrowserRuntimeConfig, BrowserSessionSpec,
    DEFAULT_BROWSER_PROFILE_ID,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::{Child, Command};

pub(super) struct LocalBrowserProcess {
    pub(super) child: Child,
    pub(super) browser_websocket_url: String,
    pub(super) download_dir: PathBuf,
}

pub(super) fn browser_profile_storage_root(
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
    pub(super) async fn start(
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

    pub(super) async fn shutdown(&mut self) -> Result<(), BrowserError> {
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

pub(super) fn discover_browser_executable(
    configured: Option<&Path>,
) -> Result<PathBuf, BrowserError> {
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
