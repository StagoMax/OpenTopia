//! Product-owned runtime for Office document libraries.
//!
//! The model never discovers an arbitrary Python from `PATH`.  A packaged
//! desktop build supplies `OPENTOPIA_OFFICE_RUNTIME_ROOT`, which points at a
//! versioned runtime directory with this module's manifest.  Developers may
//! opt in to the same layout with the environment variable; the older
//! `OPENTOPIA_SPREADSHEET_PYTHON` override remains only as a migration bridge.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use wait_timeout::ChildExt;

pub const OFFICE_RUNTIME_MANIFEST: &str = "office-runtime.json";
pub const OFFICE_RUNTIME_ID: &str = "ai.opentopia.office-runtime";
const PYTHON_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfficeRuntimeSource {
    Managed {
        root: PathBuf,
        version: String,
    },
    /// Temporary compatibility bridge for source builds.  It is deliberately
    /// never discovered from PATH and is not used by packaged desktop builds.
    LegacyOverride {
        executable: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficePythonRuntime {
    pub executable: PathBuf,
    pub source: OfficeRuntimeSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfficeRuntimeStatus {
    Ready {
        version: String,
        root: PathBuf,
        openpyxl_version: String,
    },
    LegacyOverride {
        executable: PathBuf,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum OfficeRuntimeError {
    #[error("Office runtime is unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OfficeRuntimeManifest {
    schema_version: u32,
    id: String,
    version: String,
    python: OfficePythonManifest,
    packages: OfficeRuntimePackages,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OfficePythonManifest {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OfficeRuntimePackages {
    openpyxl: String,
}

/// Resolves and probes a single product-owned Office runtime.
///
/// Keep this boundary independent of spreadsheet implementation details so a
/// future document/PDF backend can reuse the same packaged runtime contract.
pub struct OfficeRuntime {
    configured_root: Option<PathBuf>,
    legacy_python: Option<PathBuf>,
    resolved: OnceLock<Result<OfficePythonRuntime, String>>,
}

impl OfficeRuntime {
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<OfficeRuntime>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::from_environment())))
    }

    pub fn from_environment() -> Self {
        let configured_root = env::var_os("OPENTOPIA_OFFICE_RUNTIME_ROOT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(default_packaged_runtime_root);
        let legacy_python = env::var_os("OPENTOPIA_SPREADSHEET_PYTHON")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self {
            configured_root,
            legacy_python,
            resolved: OnceLock::new(),
        }
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self {
            configured_root: Some(root),
            legacy_python: None,
            resolved: OnceLock::new(),
        }
    }

    pub fn python_for_openpyxl(&self) -> Result<OfficePythonRuntime, OfficeRuntimeError> {
        self.resolved
            .get_or_init(|| {
                self.resolve_python_uncached()
                    .map_err(|error| error.to_string())
            })
            .clone()
            .map_err(OfficeRuntimeError::Unavailable)
    }

    pub fn status(&self) -> OfficeRuntimeStatus {
        match self.python_for_openpyxl() {
            Ok(runtime) => match runtime.source {
                OfficeRuntimeSource::Managed { root, version } => {
                    let manifest = read_manifest(&root).ok();
                    OfficeRuntimeStatus::Ready {
                        version,
                        root,
                        openpyxl_version: manifest
                            .map(|manifest| manifest.packages.openpyxl)
                            .unwrap_or_else(|| "unknown".to_string()),
                    }
                }
                OfficeRuntimeSource::LegacyOverride { executable } => {
                    OfficeRuntimeStatus::LegacyOverride { executable }
                }
            },
            Err(error) => OfficeRuntimeStatus::Unavailable {
                reason: error.to_string(),
            },
        }
    }

    fn resolve_python_uncached(&self) -> Result<OfficePythonRuntime, OfficeRuntimeError> {
        if let Some(root) = self.configured_root.as_deref() {
            return resolve_managed_python(root);
        }
        if let Some(executable) = self.legacy_python.as_deref() {
            probe_openpyxl(executable, None)?;
            return Ok(OfficePythonRuntime {
                executable: executable.to_path_buf(),
                source: OfficeRuntimeSource::LegacyOverride {
                    executable: executable.to_path_buf(),
                },
            });
        }
        Err(OfficeRuntimeError::Unavailable(format!(
            "set OPENTOPIA_OFFICE_RUNTIME_ROOT to a packaged Office runtime ({OFFICE_RUNTIME_MANIFEST})"
        )))
    }
}

fn default_packaged_runtime_root() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let candidate = executable.parent()?.join("office-runtime");
    candidate.is_dir().then_some(candidate)
}

fn resolve_managed_python(root: &Path) -> Result<OfficePythonRuntime, OfficeRuntimeError> {
    let manifest = read_manifest(root)?;
    if manifest.schema_version != 1 || manifest.id != OFFICE_RUNTIME_ID {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "{} is not an OpenTopia Office runtime manifest",
            root.join(OFFICE_RUNTIME_MANIFEST).display()
        )));
    }
    if manifest.version.trim().is_empty() || manifest.packages.openpyxl.trim().is_empty() {
        return Err(OfficeRuntimeError::Unavailable(
            "Office runtime manifest has an empty version or openpyxl pin".to_string(),
        ));
    }
    let executable = resolve_relative(root, &manifest.python.path)?;
    if !executable.is_file() {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "managed Python executable is missing at {}",
            executable.display()
        )));
    }
    verify_sha256(&executable, &manifest.python.sha256)?;
    probe_openpyxl(&executable, Some(&manifest.packages.openpyxl))?;
    Ok(OfficePythonRuntime {
        executable,
        source: OfficeRuntimeSource::Managed {
            root: root.to_path_buf(),
            version: manifest.version,
        },
    })
}

fn read_manifest(root: &Path) -> Result<OfficeRuntimeManifest, OfficeRuntimeError> {
    let path = root.join(OFFICE_RUNTIME_MANIFEST);
    let content = fs::read_to_string(&path).map_err(|error| {
        OfficeRuntimeError::Unavailable(format!("cannot read {}: {error}", path.display()))
    })?;
    serde_json::from_str(&content).map_err(|error| {
        OfficeRuntimeError::Unavailable(format!("invalid {}: {error}", path.display()))
    })
}

fn resolve_relative(root: &Path, value: &str) -> Result<PathBuf, OfficeRuntimeError> {
    let relative = Path::new(value);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(OfficeRuntimeError::Unavailable(
            "Office runtime Python path must be a non-empty relative path".to_string(),
        ));
    }
    Ok(root.join(relative))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), OfficeRuntimeError> {
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OfficeRuntimeError::Unavailable(
            "Office runtime manifest has an invalid Python sha256".to_string(),
        ));
    }
    let bytes = fs::read(path).map_err(|error| {
        OfficeRuntimeError::Unavailable(format!("cannot hash {}: {error}", path.display()))
    })?;
    let actual = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "managed Python hash does not match {}",
            path.display()
        )));
    }
    Ok(())
}

fn probe_openpyxl(python: &Path, expected_version: Option<&str>) -> Result<(), OfficeRuntimeError> {
    let script =
        "import importlib.metadata; import openpyxl; print(importlib.metadata.version('openpyxl'))";
    let mut child = Command::new(python)
        .args(["-I", "-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            OfficeRuntimeError::Unavailable(format!(
                "failed to start {}: {error}",
                python.display()
            ))
        })?;
    let status = match child.wait_timeout(PYTHON_PROBE_TIMEOUT) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(OfficeRuntimeError::Unavailable(format!(
                "managed Python probe timed out after {} seconds",
                PYTHON_PROBE_TIMEOUT.as_secs()
            )));
        }
        Err(error) => {
            return Err(OfficeRuntimeError::Unavailable(format!(
                "managed Python probe failed: {error}"
            )));
        }
    };
    let output = child.wait_with_output().map_err(|error| {
        OfficeRuntimeError::Unavailable(format!("cannot read managed Python probe output: {error}"))
    })?;
    if !status.success() || !output.status.success() {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "{} cannot import openpyxl{}",
            python.display(),
            stderr_suffix(&output.stderr)
        )));
    }
    if let Some(expected) = expected_version {
        let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if actual != expected {
            return Err(OfficeRuntimeError::Unavailable(format!(
                "managed openpyxl version {actual:?} does not match manifest pin {expected:?}"
            )));
        }
    }
    Ok(())
}

fn stderr_suffix(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(": {stderr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_runtime_rejects_path_escape() {
        let error = resolve_relative(Path::new("C:/runtime"), "../python.exe")
            .expect_err("parent traversal must be rejected");
        assert!(error.to_string().contains("relative path"));
    }

    #[test]
    fn managed_runtime_rejects_invalid_hash() {
        let root =
            std::env::temp_dir().join(format!("opentopia-office-runtime-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create runtime root");
        fs::write(
            root.join(OFFICE_RUNTIME_MANIFEST),
            r#"{"schemaVersion":1,"id":"ai.opentopia.office-runtime","version":"1","python":{"path":"python.exe","sha256":"not-a-hash"},"packages":{"openpyxl":"3.1.5"}}"#,
        )
        .expect("write manifest");
        fs::write(root.join("python.exe"), b"test").expect("write placeholder");
        let error =
            resolve_managed_python(&root).expect_err("invalid hash must fail before execution");
        assert!(error.to_string().contains("invalid Python sha256"));
        fs::remove_dir_all(root).expect("remove test runtime");
    }
}
