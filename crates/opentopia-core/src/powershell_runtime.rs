use crate::execution_spec::ShellDialect;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use uuid::Uuid;
use wait_timeout::ChildExt;
use zip::ZipArchive;

pub const MANAGED_POWERSHELL_VERSION: &str = "7.6.5";

const MAX_ARCHIVE_BYTES: u64 = 250 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 750 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 2_000;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellRuntimeSource {
    Configured,
    Managed,
    StandardInstall,
    Path,
    System,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShellRuntime {
    pub program: PathBuf,
    pub dialect: ShellDialect,
    pub version: Option<String>,
    pub source: ShellRuntimeSource,
}

impl ShellRuntime {
    pub fn runtime_read_roots(&self) -> Vec<PathBuf> {
        self.program
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| vec![parent.to_path_buf()])
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedPowerShellStatus {
    NotRequired,
    Pending,
    Downloading,
    Ready,
    Disabled,
    Failed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShellRuntimeStatus {
    pub runtime: ShellRuntime,
    pub managed_version: &'static str,
    pub managed_status: ManagedPowerShellStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ShellRuntimeState {
    runtime: ShellRuntime,
    managed_status: ManagedPowerShellStatus,
    managed_error: Option<String>,
}

static SHELL_RUNTIME: OnceLock<RwLock<ShellRuntimeState>> = OnceLock::new();

pub fn initialize_shell_runtime() -> ShellRuntimeStatus {
    let runtime = resolve_shell_runtime();
    let managed_status = initial_managed_status(&runtime);
    let state = shell_runtime_state();
    *state.write().expect("shell runtime lock poisoned") = ShellRuntimeState {
        runtime,
        managed_status,
        managed_error: None,
    };
    current_shell_runtime_status()
}

pub fn current_shell_runtime() -> ShellRuntime {
    shell_runtime_state()
        .read()
        .expect("shell runtime lock poisoned")
        .runtime
        .clone()
}

pub fn current_shell_runtime_status() -> ShellRuntimeStatus {
    let state = shell_runtime_state()
        .read()
        .expect("shell runtime lock poisoned");
    ShellRuntimeStatus {
        runtime: state.runtime.clone(),
        managed_version: MANAGED_POWERSHELL_VERSION,
        managed_status: state.managed_status,
        managed_error: state.managed_error.clone(),
    }
}

pub fn start_managed_powershell_install() -> Option<tokio::task::JoinHandle<()>> {
    let status = current_shell_runtime_status();
    if !cfg!(windows)
        || status.runtime.dialect == ShellDialect::PowerShell7
        || status.managed_status == ManagedPowerShellStatus::Downloading
        || !managed_install_enabled()
    {
        return None;
    }
    set_managed_status(ManagedPowerShellStatus::Downloading, None);
    Some(tokio::spawn(async {
        match ensure_managed_powershell().await {
            Ok(runtime) => {
                info!(
                    version = runtime.version.as_deref().unwrap_or("unknown"),
                    path = %runtime.program.display(),
                    "managed PowerShell runtime is ready"
                );
                let state = shell_runtime_state();
                *state.write().expect("shell runtime lock poisoned") = ShellRuntimeState {
                    runtime,
                    managed_status: ManagedPowerShellStatus::Ready,
                    managed_error: None,
                };
            }
            Err(error) => {
                warn!(%error, "managed PowerShell installation failed; keeping Windows PowerShell fallback");
                set_managed_status(ManagedPowerShellStatus::Failed, Some(format!("{error:#}")));
            }
        }
    }))
}

fn shell_runtime_state() -> &'static RwLock<ShellRuntimeState> {
    SHELL_RUNTIME.get_or_init(|| {
        let runtime = resolve_shell_runtime();
        RwLock::new(ShellRuntimeState {
            managed_status: initial_managed_status(&runtime),
            runtime,
            managed_error: None,
        })
    })
}

fn initial_managed_status(runtime: &ShellRuntime) -> ManagedPowerShellStatus {
    if !cfg!(windows) || runtime.dialect == ShellDialect::PowerShell7 {
        if runtime.source == ShellRuntimeSource::Managed {
            ManagedPowerShellStatus::Ready
        } else {
            ManagedPowerShellStatus::NotRequired
        }
    } else if managed_install_enabled() {
        ManagedPowerShellStatus::Pending
    } else {
        ManagedPowerShellStatus::Disabled
    }
}

fn set_managed_status(status: ManagedPowerShellStatus, error: Option<String>) {
    let mut state = shell_runtime_state()
        .write()
        .expect("shell runtime lock poisoned");
    state.managed_status = status;
    state.managed_error = error;
}

fn managed_install_enabled() -> bool {
    env::var("OPENTOPIA_POWERSHELL_AUTO_INSTALL")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

fn resolve_shell_runtime() -> ShellRuntime {
    if !cfg!(windows) {
        return ShellRuntime {
            program: PathBuf::from("sh"),
            dialect: ShellDialect::PosixSh,
            version: None,
            source: ShellRuntimeSource::System,
        };
    }

    for (source, candidate) in powershell_7_candidates() {
        if let Some(version) = probe_powershell_version(&candidate).filter(|version| {
            version
                .split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
                .is_some_and(|major| major >= 7)
        }) {
            return ShellRuntime {
                program: candidate,
                dialect: ShellDialect::PowerShell7,
                version: Some(version),
                source,
            };
        }
    }

    let program = windows_powershell_path();
    ShellRuntime {
        version: probe_powershell_version(&program),
        program,
        dialect: ShellDialect::WindowsPowerShell51,
        source: ShellRuntimeSource::System,
    }
}

fn powershell_7_candidates() -> Vec<(ShellRuntimeSource, PathBuf)> {
    let mut candidates = Vec::new();
    if let Some(configured) = env::var_os("OPENTOPIA_POWERSHELL_PATH") {
        let path = PathBuf::from(configured);
        candidates.push((
            ShellRuntimeSource::Configured,
            if path.is_dir() {
                path.join("pwsh.exe")
            } else {
                path
            },
        ));
    }
    candidates.push((ShellRuntimeSource::Managed, managed_powershell_executable()));
    for variable in ["ProgramFiles", "ProgramW6432"] {
        if let Some(root) = env::var_os(variable) {
            candidates.push((
                ShellRuntimeSource::StandardInstall,
                PathBuf::from(root)
                    .join("PowerShell")
                    .join("7")
                    .join("pwsh.exe"),
            ));
        }
    }
    if let Some(path) = env::var_os("PATH") {
        for entry in env::split_paths(&path).filter(|entry| entry.is_absolute()) {
            candidates.push((ShellRuntimeSource::Path, entry.join("pwsh.exe")));
        }
    }

    let mut seen = HashSet::new();
    candidates.retain(|(_, path)| path.is_file() && seen.insert(path_key(path)));
    candidates
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

fn windows_powershell_path() -> PathBuf {
    env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| {
            root.join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

fn probe_powershell_version(program: &Path) -> Option<String> {
    if program.is_absolute() && !program.is_file() {
        return None;
    }
    let mut child = Command::new(program)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let status = match child.wait_timeout(Duration::from_secs(5)).ok()? {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    let output = child.wait_with_output().ok()?;
    if !status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!version.is_empty()).then_some(version)
}

fn managed_runtime_root() -> PathBuf {
    if let Some(root) = env::var_os("OPENTOPIA_RUNTIME_HOME") {
        return PathBuf::from(root);
    }
    if cfg!(windows) {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("OpenTopia")
            .join("runtimes");
    }
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(env::temp_dir)
        .join("opentopia")
        .join("runtimes")
}

fn managed_powershell_directory() -> PathBuf {
    managed_runtime_root()
        .join("powershell")
        .join(MANAGED_POWERSHELL_VERSION)
        .join(
            managed_asset()
                .map(|asset| asset.arch)
                .unwrap_or("unsupported"),
        )
}

fn managed_powershell_executable() -> PathBuf {
    managed_powershell_directory().join("pwsh.exe")
}

#[derive(Debug, Clone, Copy)]
struct ManagedAsset {
    arch: &'static str,
    file_name: &'static str,
    sha256: &'static str,
}

fn managed_asset() -> Option<ManagedAsset> {
    match env::consts::ARCH {
        "x86_64" => Some(ManagedAsset {
            arch: "win-x64",
            file_name: "PowerShell-7.6.5-win-x64.zip",
            sha256: "32EB8F6CDCE08F86E987D625A2733E54AC3E289AE7E1621B14C0B5BCEC2434EA",
        }),
        "aarch64" => Some(ManagedAsset {
            arch: "win-arm64",
            file_name: "PowerShell-7.6.5-win-arm64.zip",
            sha256: "20514A755D16428DC4355C85E0883C859531E71CC3E122670AA1FCCDBF96BA7E",
        }),
        _ => None,
    }
}

async fn ensure_managed_powershell() -> Result<ShellRuntime> {
    let asset =
        managed_asset().context("managed PowerShell is unsupported on this architecture")?;
    let executable = managed_powershell_executable();
    if let Some(version) = probe_powershell_version(&executable) {
        return Ok(managed_runtime(executable, version));
    }

    let root = managed_runtime_root();
    tokio::fs::create_dir_all(&root)
        .await
        .with_context(|| format!("failed to create runtime directory {}", root.display()))?;
    let (archive, downloaded_archive) =
        if let Some(offline_archive) = env::var_os("OPENTOPIA_POWERSHELL_ARCHIVE") {
            (PathBuf::from(offline_archive), false)
        } else {
            (download_managed_archive(&root, asset).await?, true)
        };
    if let Err(error) = verify_archive_hash(&archive, asset.sha256).await {
        if downloaded_archive {
            let _ = tokio::fs::remove_file(&archive).await;
        }
        return Err(error);
    }

    let staging = root.join(format!(
        ".powershell-{}-{}",
        MANAGED_POWERSHELL_VERSION,
        Uuid::new_v4()
    ));
    let final_directory = managed_powershell_directory();
    let archive_for_extract = archive.clone();
    let staging_for_extract = staging.clone();
    let extraction = tokio::task::spawn_blocking(move || {
        extract_archive(&archive_for_extract, &staging_for_extract)
    })
    .await
    .context("managed PowerShell extraction task failed")?;
    if let Err(error) = extraction {
        let _ = tokio::fs::remove_dir_all(&staging).await;
        if downloaded_archive {
            let _ = tokio::fs::remove_file(&archive).await;
        }
        return Err(error);
    }

    let activation = async {
        let staged_executable = staging.join("pwsh.exe");
        let version = probe_powershell_version(&staged_executable)
            .context("downloaded PowerShell runtime did not contain a working pwsh.exe")?;
        anyhow::ensure!(
            version == MANAGED_POWERSHELL_VERSION,
            "downloaded PowerShell version {version} did not match pinned version {MANAGED_POWERSHELL_VERSION}"
        );

        if let Some(parent) = final_directory.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if final_directory.exists()
            && probe_powershell_version(&final_directory.join("pwsh.exe")).as_deref()
                != Some(MANAGED_POWERSHELL_VERSION)
        {
            tokio::fs::remove_dir_all(&final_directory)
                .await
                .context("failed to replace an invalid managed PowerShell runtime")?;
        }
        match tokio::fs::rename(&staging, &final_directory).await {
            Ok(()) => {}
            Err(error)
                if probe_powershell_version(&final_directory.join("pwsh.exe")).as_deref()
                    == Some(MANAGED_POWERSHELL_VERSION) =>
            {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                warn!(%error, "another process installed the managed PowerShell runtime first");
            }
            Err(error) => {
                return Err(error).context("failed to activate managed PowerShell runtime")
            }
        }
        let version = probe_powershell_version(&executable)
            .context("activated managed PowerShell runtime failed its version probe")?;
        Ok(managed_runtime(executable, version))
    }
    .await;
    if activation.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    if downloaded_archive {
        let _ = tokio::fs::remove_file(&archive).await;
    }
    activation
}

fn managed_runtime(program: PathBuf, version: String) -> ShellRuntime {
    ShellRuntime {
        program,
        dialect: ShellDialect::PowerShell7,
        version: Some(version),
        source: ShellRuntimeSource::Managed,
    }
}

async fn download_managed_archive(root: &Path, asset: ManagedAsset) -> Result<PathBuf> {
    let url = format!(
        "https://github.com/PowerShell/PowerShell/releases/download/v{MANAGED_POWERSHELL_VERSION}/{}",
        asset.file_name
    );
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(15 * 60))
        .user_agent("OpenTopia managed runtime installer")
        .build()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to download {url}"))?
        .error_for_status()
        .with_context(|| format!("PowerShell download returned an error for {url}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        anyhow::bail!("PowerShell archive exceeds the configured download limit");
    }

    let path = root.join(format!(".{}-{}.download", asset.file_name, Uuid::new_v4()));
    let mut output = tokio::fs::File::create(&path).await?;
    let mut downloaded = 0_u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("failed while downloading PowerShell archive")?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_ARCHIVE_BYTES {
            let _ = tokio::fs::remove_file(&path).await;
            anyhow::bail!("PowerShell archive exceeds the configured download limit");
        }
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    Ok(path)
}

async fn verify_archive_hash(path: &Path, expected: &str) -> Result<()> {
    let path = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || -> Result<String> {
        let mut file = fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 128 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect())
    })
    .await
    .context("PowerShell archive hash task failed")??;
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(expected),
        "PowerShell archive SHA-256 mismatch: expected {expected}, got {actual}"
    );
    Ok(())
}

fn extract_archive(archive_path: &Path, output_root: &Path) -> Result<()> {
    fs::create_dir_all(output_root)?;
    let file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).context("invalid PowerShell ZIP archive")?;
    anyhow::ensure!(
        archive.len() <= MAX_ARCHIVE_ENTRIES,
        "PowerShell archive contains too many entries"
    );
    let mut extracted = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .context("PowerShell archive contains an unsafe path")?
            .to_path_buf();
        extracted = extracted.saturating_add(entry.size());
        anyhow::ensure!(
            extracted <= MAX_EXTRACTED_BYTES,
            "PowerShell archive exceeds the configured extraction limit"
        );
        let destination = output_root.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::File::create(&destination)?;
        std::io::copy(&mut entry, &mut output)?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_asset_is_pinned_for_supported_windows_architectures() {
        if matches!(env::consts::ARCH, "x86_64" | "aarch64") {
            let asset = managed_asset().expect("supported asset");
            assert!(asset.file_name.contains(MANAGED_POWERSHELL_VERSION));
            assert_eq!(asset.sha256.len(), 64);
        }
    }

    #[test]
    fn managed_runtime_location_is_versioned() {
        let executable = managed_powershell_executable();
        assert!(executable
            .to_string_lossy()
            .contains(MANAGED_POWERSHELL_VERSION));
        assert_eq!(
            executable.file_name(),
            Some(std::ffi::OsStr::new("pwsh.exe"))
        );
    }
}
