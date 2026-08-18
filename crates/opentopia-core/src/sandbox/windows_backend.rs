//! OpenTopia Windows helper discovery, protocol verification, and setup lifecycle.

use super::contract::{
    OsSandboxPlatform, WindowsSandboxSetupComponents, WindowsSandboxSetupState,
    WindowsSandboxSetupStatus,
};
use anyhow::Context;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

static WINDOWS_SANDBOX_PROTOCOL_CACHE: OnceLock<Mutex<HashMap<String, Result<(), String>>>> =
    OnceLock::new();

fn sandbox_binary_fingerprint(path: &Path) -> anyhow::Result<String> {
    let metadata = std::fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(format!(
        "{}\n{}\n{modified}",
        path.display(),
        metadata.len()
    ))
}

fn verify_opentopia_sandbox_binary(path: &Path) -> anyhow::Result<()> {
    let fingerprint = sandbox_binary_fingerprint(path)?;
    let cache = WINDOWS_SANDBOX_PROTOCOL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(result) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&fingerprint).cloned())
    {
        return result.map_err(anyhow::Error::msg);
    }

    let result = (|| -> anyhow::Result<()> {
        let output = Command::new(path)
            .args(["protocol", "--json"])
            .output()
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to start OpenTopia sandbox protocol handshake at '{}': {error}",
                    path.display()
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "OpenTopia sandbox helper at '{}' does not implement the required protocol handshake (exit {}): {}. Rebuild the server and helper as one runtime bundle.",
                path.display(),
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() { "no diagnostic" } else { &stderr }
            );
        }
        let info: opentopia_sandbox_protocol::SandboxProtocolInfo =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                anyhow::anyhow!(
                    "OpenTopia sandbox helper at '{}' returned an invalid protocol descriptor: {error}",
                    path.display()
                )
            })?;
        if let Some(error) = info.compatibility_error() {
            anyhow::bail!(
                "OpenTopia sandbox helper at '{}' is incompatible: {error}. Rebuild the server and helper as one runtime bundle.",
                path.display()
            );
        }
        Ok(())
    })()
    .map_err(|error| error.to_string());

    if let Ok(mut cache) = cache.lock() {
        cache.insert(fingerprint, result.clone());
    }
    result.map_err(anyhow::Error::msg)
}

pub(super) fn resolve_opentopia_sandbox_binary() -> anyhow::Result<Option<PathBuf>> {
    let configured = std::env::var_os("OPENTOPIA_WINDOWS_SANDBOX_BIN").map(PathBuf::from);
    if let Some(configured) = configured {
        if !configured.is_file() {
            anyhow::bail!(
                "configured OpenTopia Windows sandbox helper was not found at '{}'",
                configured.display()
            );
        }
        verify_opentopia_sandbox_binary(&configured)?;
        return Ok(Some(configured));
    }
    let (sibling, cargo_debug_sibling) = std::env::current_exe()
        .ok()
        .map(|path| {
            let sibling = path
                .parent()
                .map(|parent| parent.join("opentopia-sandbox.exe"));
            // `cargo test` runs binaries from `target/<profile>/deps`; the
            // first-party helper is emitted one directory above it.
            let cargo_debug_sibling = path.parent().and_then(|parent| {
                parent
                    .parent()
                    .map(|target_profile| target_profile.join("opentopia-sandbox.exe"))
            });
            (sibling, cargo_debug_sibling)
        })
        .unwrap_or((None, None));
    let candidates = sibling
        .into_iter()
        .chain(cargo_debug_sibling)
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    let mut incompatibilities = Vec::new();
    for candidate in candidates {
        match verify_opentopia_sandbox_binary(&candidate) {
            Ok(()) => return Ok(Some(candidate)),
            Err(error) => incompatibilities.push(error.to_string()),
        }
    }
    if incompatibilities.is_empty() {
        Ok(None)
    } else {
        anyhow::bail!(incompatibilities.join("; "))
    }
}

pub fn windows_sandbox_setup_status() -> anyhow::Result<WindowsSandboxSetupStatus> {
    if OsSandboxPlatform::current() != OsSandboxPlatform::Windows {
        return Ok(WindowsSandboxSetupStatus {
            supported: false,
            helper_available: false,
            state: WindowsSandboxSetupState::Unavailable,
            backend: "dedicated_user".to_string(),
            state_dir: None,
            components: WindowsSandboxSetupComponents::default(),
            issues: vec!["the dedicated-user sandbox backend is available only on Windows".into()],
        });
    }
    let Some(helper) = resolve_opentopia_sandbox_binary()? else {
        return Ok(WindowsSandboxSetupStatus {
            supported: true,
            helper_available: false,
            state: WindowsSandboxSetupState::Unavailable,
            backend: "dedicated_user".to_string(),
            state_dir: None,
            components: WindowsSandboxSetupComponents::default(),
            issues: vec!["the OpenTopia Windows sandbox helper was not found".into()],
        });
    };
    let output = Command::new(&helper)
        .args(["setup", "--status", "--json"])
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to query Windows sandbox setup through '{}': {error}",
                helper.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows sandbox setup status failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let status: opentopia_sandbox_protocol::SandboxSetupStatus =
        serde_json::from_slice(&output.stdout).context("parse Windows sandbox setup status")?;
    if let Some(error) = status.compatibility_error() {
        anyhow::bail!("incompatible Windows sandbox setup status: {error}");
    }
    Ok(WindowsSandboxSetupStatus {
        supported: true,
        helper_available: true,
        state: match status.state {
            opentopia_sandbox_protocol::SandboxSetupState::NotConfigured => {
                WindowsSandboxSetupState::NotConfigured
            }
            opentopia_sandbox_protocol::SandboxSetupState::Ready => WindowsSandboxSetupState::Ready,
            opentopia_sandbox_protocol::SandboxSetupState::Degraded => {
                WindowsSandboxSetupState::Degraded
            }
        },
        backend: "dedicated_user".to_string(),
        state_dir: Some(status.state_dir),
        components: WindowsSandboxSetupComponents {
            credentials: status.components.credentials,
            offline_identity: status.components.offline_identity,
            online_identity: status.components.online_identity,
            offline_network_policy: status.components.offline_network_policy,
        },
        issues: status.issues,
    })
}

pub fn setup_windows_sandbox() -> anyhow::Result<WindowsSandboxSetupStatus> {
    run_windows_sandbox_lifecycle("setup", WindowsSandboxSetupState::Ready)
}

pub fn remove_windows_sandbox() -> anyhow::Result<WindowsSandboxSetupStatus> {
    run_windows_sandbox_lifecycle("teardown", WindowsSandboxSetupState::NotConfigured)
}

fn run_windows_sandbox_lifecycle(
    action: &str,
    expected_state: WindowsSandboxSetupState,
) -> anyhow::Result<WindowsSandboxSetupStatus> {
    anyhow::ensure!(
        OsSandboxPlatform::current() == OsSandboxPlatform::Windows,
        "the dedicated-user sandbox backend can be managed only on Windows"
    );
    let helper = resolve_opentopia_sandbox_binary()?
        .context("the OpenTopia Windows sandbox helper was not found")?;
    let output = Command::new(&helper)
        .arg(action)
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to start Windows sandbox {action} through '{}': {error}",
                helper.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows sandbox {action} failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let status = windows_sandbox_setup_status()?;
    anyhow::ensure!(
        status.state == expected_state,
        "Windows sandbox {action} exited successfully but reached state {:?}: {}",
        status.state,
        status.issues.join("; ")
    );
    Ok(status)
}
