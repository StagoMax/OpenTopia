//! Product-owned Python runtime for Office document libraries.
//!
//! OpenTopia never discovers Python from `PATH`. Packaged builds use a
//! relocatable, versioned runtime, while development and recovery installs use
//! the same pinned artifact lock and managed-runtime cache. The legacy explicit
//! Python override remains a migration bridge only.

mod installer;
mod manifest;
mod runtime_lock;

use manifest::{OfficeRuntimeManifest, OFFICE_RUNTIME_MANIFEST_SCHEMA_VERSION};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use tracing::{info, warn};
use wait_timeout::ChildExt;

pub const OFFICE_RUNTIME_MANIFEST: &str = "office-runtime.json";
pub const OFFICE_RUNTIME_ID: &str = "ai.opentopia.office-runtime";
const PYTHON_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OfficeRuntimeSource {
    Configured,
    Packaged,
    Managed,
    LegacyOverride,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficePythonRuntime {
    pub executable: PathBuf,
    pub root: PathBuf,
    pub runtime_version: String,
    pub python_version: String,
    pub openpyxl_version: String,
    pub source: OfficeRuntimeSource,
}

impl OfficePythonRuntime {
    pub fn runtime_read_roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedOfficeRuntimeStatus {
    NotRequired,
    Pending,
    Downloading,
    Ready,
    Disabled,
    Failed,
}

#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficeRuntimeStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<OfficePythonRuntime>,
    pub managed_version: String,
    pub managed_status: ManagedOfficeRuntimeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_error: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum OfficeRuntimeError {
    #[error("Office runtime is unavailable: {0}")]
    Unavailable(String),
}

#[derive(Debug, Clone)]
struct OfficeRuntimeState {
    runtime: Option<OfficePythonRuntime>,
    managed_status: ManagedOfficeRuntimeStatus,
    managed_error: Option<String>,
}

pub struct OfficeRuntime {
    auto_install: bool,
    state: RwLock<OfficeRuntimeState>,
}

impl OfficeRuntime {
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<OfficeRuntime>> = OnceLock::new();
        Arc::clone(SHARED.get_or_init(|| Arc::new(Self::from_environment())))
    }

    pub fn from_environment() -> Self {
        let configured_root = env::var_os("OPENTOPIA_OFFICE_RUNTIME_ROOT")
            .filter(|value| !value.is_empty())
            .map(|root| (PathBuf::from(root), OfficeRuntimeSource::Configured))
            .or_else(|| {
                default_packaged_runtime_root().map(|root| (root, OfficeRuntimeSource::Packaged))
            });
        let legacy_python = env::var_os("OPENTOPIA_SPREADSHEET_PYTHON")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self::new(configured_root, legacy_python, managed_install_enabled())
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self::new(Some((root, OfficeRuntimeSource::Configured)), None, false)
    }

    fn new(
        configured_root: Option<(PathBuf, OfficeRuntimeSource)>,
        legacy_python: Option<PathBuf>,
        auto_install: bool,
    ) -> Self {
        let state = resolve_initial_state(
            configured_root.as_ref(),
            legacy_python.as_deref(),
            auto_install,
        );
        Self {
            auto_install,
            state: RwLock::new(state),
        }
    }

    pub fn python_for_openpyxl(&self) -> Result<OfficePythonRuntime, OfficeRuntimeError> {
        let state = self.state.read().expect("Office runtime lock poisoned");
        state.runtime.clone().ok_or_else(|| {
            OfficeRuntimeError::Unavailable(
                state
                    .managed_error
                    .clone()
                    .unwrap_or_else(|| status_unavailable_reason(state.managed_status)),
            )
        })
    }

    pub fn status(&self) -> OfficeRuntimeStatus {
        let state = self.state.read().expect("Office runtime lock poisoned");
        OfficeRuntimeStatus {
            runtime: state.runtime.clone(),
            managed_version: managed_version(),
            managed_status: state.managed_status,
            managed_error: state.managed_error.clone(),
        }
    }

    fn begin_install(&self) -> bool {
        if !self.auto_install || runtime_lock::current_python_asset().is_err() {
            return false;
        }
        let mut state = self.state.write().expect("Office runtime lock poisoned");
        if state.runtime.is_some()
            || state.managed_status == ManagedOfficeRuntimeStatus::Downloading
        {
            return false;
        }
        state.managed_status = ManagedOfficeRuntimeStatus::Downloading;
        state.managed_error = None;
        true
    }

    fn complete_install(&self, runtime: OfficePythonRuntime) {
        *self.state.write().expect("Office runtime lock poisoned") = OfficeRuntimeState {
            runtime: Some(runtime),
            managed_status: ManagedOfficeRuntimeStatus::Ready,
            managed_error: None,
        };
    }

    fn fail_install(&self, error: String) {
        let mut state = self.state.write().expect("Office runtime lock poisoned");
        state.managed_status = ManagedOfficeRuntimeStatus::Failed;
        state.managed_error = Some(error);
    }
}

pub fn initialize_office_runtime() -> OfficeRuntimeStatus {
    OfficeRuntime::shared().status()
}

pub fn current_office_runtime_status() -> OfficeRuntimeStatus {
    OfficeRuntime::shared().status()
}

pub fn start_managed_office_runtime_install() -> Option<tokio::task::JoinHandle<()>> {
    let runtime = OfficeRuntime::shared();
    if !runtime.begin_install() {
        return None;
    }
    Some(tokio::spawn(async move {
        match installer::ensure_managed_office_runtime().await {
            Ok(ready) => {
                info!(
                    python_version = %ready.python_version,
                    openpyxl_version = %ready.openpyxl_version,
                    path = %ready.executable.display(),
                    "managed Office Python runtime is ready"
                );
                runtime.complete_install(ready);
            }
            Err(error) => {
                warn!(%error, "managed Office Python installation failed");
                runtime.fail_install(format!("{error:#}"));
            }
        }
    }))
}

pub fn retry_managed_office_runtime_install() -> OfficeRuntimeStatus {
    drop(start_managed_office_runtime_install());
    current_office_runtime_status()
}

fn resolve_initial_state(
    configured_root: Option<&(PathBuf, OfficeRuntimeSource)>,
    legacy_python: Option<&Path>,
    auto_install: bool,
) -> OfficeRuntimeState {
    let mut diagnostics = Vec::new();
    if let Some((root, source)) = configured_root {
        match resolve_runtime_at(root, *source) {
            Ok(runtime) => {
                return OfficeRuntimeState {
                    runtime: Some(runtime),
                    managed_status: ManagedOfficeRuntimeStatus::NotRequired,
                    managed_error: None,
                }
            }
            Err(error) => diagnostics.push(error.to_string()),
        }
    }
    match installer::managed_office_runtime_directory() {
        Ok(root) if root.is_dir() => {
            match resolve_runtime_at(&root, OfficeRuntimeSource::Managed) {
                Ok(runtime) => {
                    return OfficeRuntimeState {
                        runtime: Some(runtime),
                        managed_status: ManagedOfficeRuntimeStatus::Ready,
                        managed_error: None,
                    }
                }
                Err(error) => diagnostics.push(error.to_string()),
            }
        }
        Ok(_) => {}
        Err(error) => diagnostics.push(error.to_string()),
    }
    if let Some(executable) = legacy_python {
        match resolve_legacy_python(executable) {
            Ok(runtime) => {
                return OfficeRuntimeState {
                    runtime: Some(runtime),
                    managed_status: ManagedOfficeRuntimeStatus::NotRequired,
                    managed_error: None,
                }
            }
            Err(error) => diagnostics.push(error.to_string()),
        }
    }

    let supported = runtime_lock::current_python_asset().is_ok();
    let managed_status = if auto_install && supported {
        ManagedOfficeRuntimeStatus::Pending
    } else {
        ManagedOfficeRuntimeStatus::Disabled
    };
    OfficeRuntimeState {
        runtime: None,
        managed_status,
        managed_error: (!diagnostics.is_empty()).then(|| diagnostics.join("; ")),
    }
}

fn managed_install_enabled() -> bool {
    env::var("OPENTOPIA_OFFICE_RUNTIME_AUTO_INSTALL")
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
        .unwrap_or(true)
}

fn managed_version() -> String {
    runtime_lock::runtime_lock()
        .map(|lock| lock.runtime_version.clone())
        .unwrap_or_else(|_| "unavailable".to_string())
}

fn status_unavailable_reason(status: ManagedOfficeRuntimeStatus) -> String {
    match status {
        ManagedOfficeRuntimeStatus::Pending => {
            "managed Office Python is waiting to be installed".to_string()
        }
        ManagedOfficeRuntimeStatus::Downloading => {
            "managed Office Python is still installing".to_string()
        }
        ManagedOfficeRuntimeStatus::Disabled => {
            "managed Office Python installation is disabled or unsupported".to_string()
        }
        ManagedOfficeRuntimeStatus::Failed => {
            "managed Office Python installation failed".to_string()
        }
        ManagedOfficeRuntimeStatus::Ready | ManagedOfficeRuntimeStatus::NotRequired => {
            "Office Python runtime is unavailable".to_string()
        }
    }
}

fn default_packaged_runtime_root() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let candidate = executable.parent()?.join("office-runtime");
    candidate.is_dir().then_some(candidate)
}

pub(super) fn resolve_runtime_at(
    root: &Path,
    source: OfficeRuntimeSource,
) -> Result<OfficePythonRuntime, OfficeRuntimeError> {
    let manifest = read_manifest(root)?;
    if !matches!(
        manifest.schema_version,
        1 | OFFICE_RUNTIME_MANIFEST_SCHEMA_VERSION
    ) || manifest.id != OFFICE_RUNTIME_ID
    {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "{} is not a supported OpenTopia Office runtime manifest",
            root.join(OFFICE_RUNTIME_MANIFEST).display()
        )));
    }
    if manifest.version.trim().is_empty() || manifest.packages.openpyxl.trim().is_empty() {
        return Err(OfficeRuntimeError::Unavailable(
            "Office runtime manifest has empty version metadata".to_string(),
        ));
    }
    if manifest.schema_version == OFFICE_RUNTIME_MANIFEST_SCHEMA_VERSION {
        validate_v2_manifest(&manifest)?;
    }
    let executable = resolve_relative(root, &manifest.python.path)?;
    if !executable.is_file() {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "managed Python executable is missing at {}",
            executable.display()
        )));
    }
    verify_sha256(&executable, &manifest.python.sha256)?;
    let probe = probe_python(&executable)?;
    if let Some(expected) = manifest.python.version.as_deref() {
        if probe.python != expected {
            return Err(OfficeRuntimeError::Unavailable(format!(
                "managed Python version {:?} does not match manifest pin {:?}",
                probe.python, expected
            )));
        }
    }
    if probe.openpyxl != manifest.packages.openpyxl {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "managed openpyxl version {:?} does not match manifest pin {:?}",
            probe.openpyxl, manifest.packages.openpyxl
        )));
    }
    Ok(OfficePythonRuntime {
        executable,
        root: root.to_path_buf(),
        runtime_version: manifest.version,
        python_version: probe.python,
        openpyxl_version: probe.openpyxl,
        source,
    })
}

fn validate_v2_manifest(manifest: &OfficeRuntimeManifest) -> Result<(), OfficeRuntimeError> {
    let target = runtime_lock::current_target_id().ok_or_else(|| {
        OfficeRuntimeError::Unavailable("current Office runtime target is unsupported".to_string())
    })?;
    if manifest.target.as_deref() != Some(target) {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "Office runtime target {:?} does not match {target:?}",
            manifest.target
        )));
    }
    if manifest.python.version.as_deref().is_none_or(str::is_empty)
        || manifest.python.distribution.is_none()
        || manifest
            .packages
            .et_xmlfile
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(OfficeRuntimeError::Unavailable(
            "Office runtime v2 manifest lacks distribution or package provenance".to_string(),
        ));
    }
    Ok(())
}

fn resolve_legacy_python(executable: &Path) -> Result<OfficePythonRuntime, OfficeRuntimeError> {
    let probe = probe_python(executable)?;
    Ok(OfficePythonRuntime {
        executable: executable.to_path_buf(),
        root: executable
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        runtime_version: "legacy-override".to_string(),
        python_version: probe.python,
        openpyxl_version: probe.openpyxl,
        source: OfficeRuntimeSource::LegacyOverride,
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
    let mut file = fs::File::open(path).map_err(|error| {
        OfficeRuntimeError::Unavailable(format!("cannot hash {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            OfficeRuntimeError::Unavailable(format!("cannot hash {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = hasher
        .finalize()
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

#[derive(Debug, Deserialize)]
struct PythonProbe {
    python: String,
    openpyxl: String,
}

fn probe_python(python: &Path) -> Result<PythonProbe, OfficeRuntimeError> {
    let script = "import importlib.metadata,json,sys; print(json.dumps({'python': '.'.join(map(str, sys.version_info[:3])), 'openpyxl': importlib.metadata.version('openpyxl')}))";
    let mut child = Command::new(python)
        .args(["-I", "-c", script])
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
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
        OfficeRuntimeError::Unavailable(format!("cannot read managed Python output: {error}"))
    })?;
    if !status.success() || !output.status.success() {
        return Err(OfficeRuntimeError::Unavailable(format!(
            "{} cannot load the pinned Office packages{}",
            python.display(),
            stderr_suffix(&output.stderr)
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        OfficeRuntimeError::Unavailable(format!(
            "{} returned an invalid runtime probe: {error}",
            python.display()
        ))
    })
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
            .expect_err("parent traversal must fail");
        assert!(error.to_string().contains("relative path"));
    }

    #[test]
    fn managed_runtime_rejects_invalid_hash_before_execution() {
        let root = TestDirectory::new();
        fs::write(
            root.path().join(OFFICE_RUNTIME_MANIFEST),
            r#"{"schemaVersion":1,"id":"ai.opentopia.office-runtime","version":"1","python":{"path":"python.exe","sha256":"not-a-hash"},"packages":{"openpyxl":"3.1.5"}}"#,
        )
        .unwrap();
        fs::write(root.path().join("python.exe"), b"test").unwrap();
        let error = resolve_runtime_at(root.path(), OfficeRuntimeSource::Configured)
            .expect_err("invalid hash must fail before execution");
        assert!(error.to_string().contains("invalid Python sha256"));
    }

    #[test]
    fn v2_manifest_rejects_a_different_target() {
        let manifest = OfficeRuntimeManifest {
            schema_version: OFFICE_RUNTIME_MANIFEST_SCHEMA_VERSION,
            id: OFFICE_RUNTIME_ID.to_string(),
            version: "test".to_string(),
            target: Some("unsupported-target".to_string()),
            python: manifest::OfficePythonManifest {
                path: "python/python.exe".to_string(),
                sha256: "0".repeat(64),
                version: Some("3.12.14".to_string()),
                distribution: Some(manifest::OfficePythonDistributionManifest {
                    provider: "test".to_string(),
                    release: "test".to_string(),
                    target_triple: "test".to_string(),
                    archive_sha256: "0".repeat(64),
                }),
            },
            packages: manifest::OfficeRuntimePackages {
                openpyxl: "3.1.5".to_string(),
                et_xmlfile: Some("2.0.0".to_string()),
            },
        };
        let error = validate_v2_manifest(&manifest).expect_err("target mismatch must fail");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn prepared_runtime_fixture_is_relocatable_when_configured() {
        let Some(root) = env::var_os("OPENTOPIA_TEST_OFFICE_RUNTIME_ROOT") else {
            return;
        };
        let runtime = resolve_runtime_at(Path::new(&root), OfficeRuntimeSource::Configured)
            .expect("prepared standalone Office runtime must resolve");
        assert_eq!(runtime.python_version, "3.12.14");
        assert_eq!(runtime.openpyxl_version, "3.1.5");
        assert!(runtime.executable.starts_with(runtime.root));
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                env::temp_dir().join(format!("opentopia-office-runtime-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
