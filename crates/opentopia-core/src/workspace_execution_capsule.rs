//! Immutable workspace-scoped environment projected into shell executions.
//!
//! A shell is only the foreground launcher; its descendants perform most real
//! work. Keeping Git policy and managed tool runtimes in one capsule makes the
//! same contract apply to foreground, background, and persistent executions
//! without parsing opaque shell source.

use anyhow::{anyhow, Context};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static MANAGED_RUNTIME_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceCapabilityIssue {
    pub capability: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceExecutionCapsule {
    environment: Vec<(OsString, OsString)>,
    path_entries: Vec<PathBuf>,
    read_roots: Vec<PathBuf>,
    managed_runtime_roots: Vec<PathBuf>,
    issues: Vec<WorkspaceCapabilityIssue>,
    fingerprint: String,
}

impl WorkspaceExecutionCapsule {
    pub(crate) fn discover(workspace_root: &Path) -> Self {
        let workspace_root = normalized_canonical_path(workspace_root);
        let mut builder = CapsuleBuilder::default();
        builder.add_git_policy(&workspace_root);
        builder.add_agent_tools_runtime();
        builder.add_rust_toolchain();
        builder.add_package_manager(&workspace_root);
        builder.finish()
    }

    pub(crate) fn environment(&self) -> &[(OsString, OsString)] {
        &self.environment
    }

    pub(crate) fn path_entries(&self) -> &[PathBuf] {
        &self.path_entries
    }

    pub(crate) fn read_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    pub(crate) fn managed_runtime_roots(&self) -> &[PathBuf] {
        &self.managed_runtime_roots
    }

    pub(crate) fn issues(&self) -> &[WorkspaceCapabilityIssue] {
        &self.issues
    }

    pub(crate) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[derive(Default)]
struct CapsuleBuilder {
    environment: BTreeMap<String, (OsString, OsString)>,
    path_entries: Vec<PathBuf>,
    read_roots: Vec<PathBuf>,
    managed_runtime_roots: Vec<PathBuf>,
    issues: Vec<WorkspaceCapabilityIssue>,
}

impl CapsuleBuilder {
    fn add_git_policy(&mut self, workspace_root: &Path) {
        // Environment-level Git config is inherited by every Git invocation,
        // including nested calls inside PowerShell, Node, and package scripts.
        // It avoids mutating the user's global config or trusting every repo.
        self.insert_env("GIT_CONFIG_COUNT", "3");
        self.insert_env("GIT_CONFIG_KEY_0", "safe.directory");
        self.insert_env("GIT_CONFIG_VALUE_0", workspace_root.as_os_str());
        self.insert_env("GIT_CONFIG_KEY_1", "core.hooksPath");
        self.insert_env(
            "GIT_CONFIG_VALUE_1",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        );
        self.insert_env("GIT_CONFIG_KEY_2", "core.fsmonitor");
        self.insert_env("GIT_CONFIG_VALUE_2", "false");
    }

    fn add_agent_tools_runtime(&mut self) {
        let Some(root) = std::env::var_os("OPENTOPIA_AGENT_TOOLS_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        if root.is_dir() {
            self.push_read_root(root);
        } else {
            self.issues.push(WorkspaceCapabilityIssue {
                capability: "agent_tools",
                reason: format!(
                    "OPENTOPIA_AGENT_TOOLS_ROOT is unavailable: {}",
                    root.display()
                ),
            });
        }
    }

    fn add_rust_toolchain(&mut self) {
        let candidate = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| host_home_directory().map(|home| home.join(".rustup")));
        let Some(root) = candidate else {
            return;
        };
        if !root.is_dir() {
            return;
        }
        let root = normalized_canonical_path(&root);
        self.insert_env("RUSTUP_HOME", root.as_os_str());
        self.push_read_root(root);
    }

    fn add_package_manager(&mut self, workspace_root: &Path) {
        let Some(version) = declared_pnpm_version(workspace_root) else {
            return;
        };

        match prepare_managed_pnpm(&version) {
            Ok(runtime) => {
                self.insert_env("COREPACK_HOME", runtime.corepack_home.as_os_str());
                self.push_path_entry(runtime.shims);
                self.push_read_root(runtime.root);
                self.push_managed_runtime_root(runtime.permission_root);
            }
            Err(error) => {
                self.issues.push(WorkspaceCapabilityIssue {
                    capability: "pnpm",
                    reason: error.to_string(),
                });
            }
        }
    }

    fn insert_env(&mut self, key: impl Into<OsString>, value: impl Into<OsString>) {
        let key = key.into();
        let normalized = key.to_string_lossy().to_ascii_uppercase();
        self.environment.insert(normalized, (key, value.into()));
    }

    fn push_path_entry(&mut self, path: PathBuf) {
        push_unique_path(&mut self.path_entries, normalized_canonical_path(&path));
    }

    fn push_read_root(&mut self, path: PathBuf) {
        push_unique_path(&mut self.read_roots, normalized_canonical_path(&path));
    }

    fn push_managed_runtime_root(&mut self, path: PathBuf) {
        push_unique_path(
            &mut self.managed_runtime_roots,
            normalized_canonical_path(&path),
        );
    }

    fn finish(self) -> WorkspaceExecutionCapsule {
        let environment = self.environment.into_values().collect::<Vec<_>>();
        let fingerprint = capsule_fingerprint(
            &environment,
            &self.path_entries,
            &self.read_roots,
            &self.managed_runtime_roots,
            &self.issues,
        );
        WorkspaceExecutionCapsule {
            environment,
            path_entries: self.path_entries,
            read_roots: self.read_roots,
            managed_runtime_roots: self.managed_runtime_roots,
            issues: self.issues,
            fingerprint,
        }
    }
}

#[derive(Debug)]
struct ManagedPnpmRuntime {
    root: PathBuf,
    permission_root: PathBuf,
    corepack_home: PathBuf,
    shims: PathBuf,
}

fn prepare_managed_pnpm(version: &str) -> anyhow::Result<ManagedPnpmRuntime> {
    validate_runtime_version(version)?;
    let state_root = managed_runtime_state_root()?
        .join("workspace-tools")
        .join("pnpm");
    prepare_managed_pnpm_at(version, &state_root, &pnpm_source_candidates(version))
}

fn prepare_managed_pnpm_at(
    version: &str,
    state_root: &Path,
    source_candidates: &[PathBuf],
) -> anyhow::Result<ManagedPnpmRuntime> {
    validate_runtime_version(version)?;
    let _guard = MANAGED_RUNTIME_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let target = state_root.join(version);
    if let Ok(runtime) = validate_managed_pnpm(&target, version) {
        return Ok(runtime);
    }

    let source = source_candidates
        .iter()
        .find(|path| validate_pnpm_package(path, version).is_ok())
        .ok_or_else(|| {
            anyhow!(
                "pnpm {version} is declared by package.json but no matching local Corepack or pnpm package is installed"
            )
        })?;

    fs::create_dir_all(state_root).with_context(|| {
        format!(
            "failed to create managed package-manager root {}",
            state_root.display()
        )
    })?;
    let staging = state_root.join(format!(".{version}.staging-{}", uuid::Uuid::new_v4()));
    let package = staging
        .join("corepack")
        .join("v1")
        .join("pnpm")
        .join(version);

    let prepared = (|| -> anyhow::Result<()> {
        copy_directory(source, &package)?;
        let metadata = validate_pnpm_package(&package, version)?;
        let shims = staging.join("shims");
        fs::create_dir_all(&shims)?;
        write_pnpm_shims(&shims, &package, &metadata)?;
        validate_managed_pnpm(&staging, version)?;
        Ok(())
    })();
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    if target.exists() {
        fs::remove_dir_all(&target).with_context(|| {
            format!(
                "failed to replace invalid managed runtime {}",
                target.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(&staging, &target) {
        if validate_managed_pnpm(&target, version).is_err() {
            let _ = fs::remove_dir_all(&staging);
            return Err(error).with_context(|| {
                format!(
                    "failed to publish managed pnpm runtime {}",
                    target.display()
                )
            });
        }
        let _ = fs::remove_dir_all(&staging);
    }
    validate_managed_pnpm(&target, version)
}

#[derive(Debug)]
struct PnpmPackageMetadata {
    pnpm_bin: PathBuf,
    pnpx_bin: PathBuf,
}

fn validate_pnpm_package(root: &Path, version: &str) -> anyhow::Result<PnpmPackageMetadata> {
    let package_json = root.join("package.json");
    let value: Value = serde_json::from_slice(
        &fs::read(&package_json)
            .with_context(|| format!("failed to read {}", package_json.display()))?,
    )
    .with_context(|| format!("invalid pnpm metadata at {}", package_json.display()))?;
    anyhow::ensure!(
        value.get("name").and_then(Value::as_str) == Some("pnpm"),
        "runtime package is not pnpm: {}",
        root.display()
    );
    anyhow::ensure!(
        value.get("version").and_then(Value::as_str) == Some(version),
        "pnpm runtime version does not match {version}: {}",
        root.display()
    );
    let bin = value
        .get("bin")
        .and_then(Value::as_object)
        .context("pnpm package metadata has no bin map")?;
    let pnpm_bin = contained_package_file(
        root,
        bin.get("pnpm")
            .and_then(Value::as_str)
            .context("pnpm package metadata has no pnpm executable")?,
    )?;
    let pnpx_bin = contained_package_file(
        root,
        bin.get("pnpx")
            .and_then(Value::as_str)
            .context("pnpm package metadata has no pnpx executable")?,
    )?;
    Ok(PnpmPackageMetadata { pnpm_bin, pnpx_bin })
}

fn validate_managed_pnpm(root: &Path, version: &str) -> anyhow::Result<ManagedPnpmRuntime> {
    let corepack_home = root.join("corepack");
    let package = corepack_home.join("v1").join("pnpm").join(version);
    validate_pnpm_package(&package, version)?;
    let shims = root.join("shims");
    let pnpm_shim = shims.join(if cfg!(windows) { "pnpm.cmd" } else { "pnpm" });
    let pnpx_shim = shims.join(if cfg!(windows) { "pnpx.cmd" } else { "pnpx" });
    anyhow::ensure!(pnpm_shim.is_file(), "managed pnpm shim is missing");
    anyhow::ensure!(pnpx_shim.is_file(), "managed pnpx shim is missing");
    #[cfg(windows)]
    {
        validate_windows_node_shim(&pnpm_shim)?;
        validate_windows_node_shim(&pnpx_shim)?;
    }
    #[cfg(not(windows))]
    {
        validate_unix_node_shim(&pnpm_shim)?;
        validate_unix_node_shim(&pnpx_shim)?;
    }
    let permission_root = normalized_canonical_path(root.parent().unwrap_or(root));
    Ok(ManagedPnpmRuntime {
        root: normalized_canonical_path(root),
        permission_root,
        corepack_home: normalized_canonical_path(&corepack_home),
        shims: normalized_canonical_path(&shims),
    })
}

fn contained_package_file(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    let relative = Path::new(relative);
    anyhow::ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "pnpm executable path escapes its package: {relative:?}"
    );
    let path = root.join(relative);
    anyhow::ensure!(
        path.is_file(),
        "pnpm executable is missing: {}",
        path.display()
    );
    Ok(path)
}

fn write_pnpm_shims(
    shims: &Path,
    package: &Path,
    metadata: &PnpmPackageMetadata,
) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        write_windows_node_shim(&shims.join("pnpm.cmd"), package, &metadata.pnpm_bin)?;
        write_windows_node_shim(&shims.join("pnpx.cmd"), package, &metadata.pnpx_bin)?;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        for (name, executable) in [("pnpm", &metadata.pnpm_bin), ("pnpx", &metadata.pnpx_bin)] {
            let path = shims.join(name);
            let relative = executable
                .strip_prefix(package)
                .context("pnpm shim target is outside its package")?;
            let version = package
                .file_name()
                .context("pnpm package has no version directory")?;
            fs::write(
                &path,
                format!(
                    "#!/bin/sh\nshim_dir=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec node \"$shim_dir/../corepack/v1/pnpm/{}/{}\" \"$@\"\n",
                    version.to_string_lossy(),
                    relative.display()
                ),
            )?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn write_windows_node_shim(path: &Path, package: &Path, executable: &Path) -> anyhow::Result<()> {
    let relative = executable
        .strip_prefix(package)
        .context("pnpm shim target is outside its package")?;
    let version = package
        .file_name()
        .context("pnpm package has no version directory")?;
    // The runtime directory is atomically renamed after staging. Resolve from
    // the shim itself so that publication cannot freeze the staging path into
    // an otherwise valid managed runtime.
    let target = PathBuf::from(r"%~dp0..")
        .join("corepack")
        .join("v1")
        .join("pnpm")
        .join(version)
        .join(relative);
    let target = target.to_string_lossy();
    fs::write(path, format!("@echo off\r\nnode \"{target}\" %*\r\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(windows)]
fn validate_windows_node_shim(path: &Path) -> anyhow::Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read managed shim {}", path.display()))?;
    anyhow::ensure!(
        contents.contains(r"%~dp0..\corepack\v1\pnpm\"),
        "managed pnpm shim is not relocatable: {}",
        path.display()
    );
    anyhow::ensure!(
        !contents.contains(".staging-"),
        "managed pnpm shim contains a staging path: {}",
        path.display()
    );
    Ok(())
}

#[cfg(not(windows))]
fn validate_unix_node_shim(path: &Path) -> anyhow::Result<()> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read managed shim {}", path.display()))?;
    anyhow::ensure!(
        contents.contains("$shim_dir/../corepack/v1/pnpm/"),
        "managed pnpm shim is not relocatable: {}",
        path.display()
    );
    anyhow::ensure!(
        !contents.contains(".staging-"),
        "managed pnpm shim contains a staging path: {}",
        path.display()
    );
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to inspect pnpm runtime {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy managed pnpm file {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            return Err(anyhow!(
                "pnpm runtime contains an unsupported filesystem entry: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn pnpm_source_candidates(version: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("OPENTOPIA_PNPM_PACKAGE_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(local)
                .join("node")
                .join("corepack")
                .join("v1")
                .join("pnpm")
                .join(version),
        );
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        candidates.push(
            PathBuf::from(roaming)
                .join("npm")
                .join("node_modules")
                .join("pnpm"),
        );
    }
    candidates
}

fn managed_runtime_state_root() -> anyhow::Result<PathBuf> {
    if let Some(root) = std::env::var_os("OPENTOPIA_RUNTIME_HOME") {
        return Ok(PathBuf::from(root));
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(root).join("OpenTopia").join("runtimes"));
    }
    #[cfg(not(windows))]
    if let Some(root) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(root).join("opentopia").join("runtimes"));
    }
    host_home_directory()
        .map(|root| root.join(".cache").join("opentopia").join("runtimes"))
        .context("cannot resolve the OpenTopia managed runtime directory")
}

fn declared_pnpm_version(workspace_root: &Path) -> Option<String> {
    let package_json = fs::read(workspace_root.join("package.json")).ok()?;
    let value: Value = serde_json::from_slice(&package_json).ok()?;
    let declaration = value.get("packageManager")?.as_str()?;
    let version = declaration.strip_prefix("pnpm@")?.split('+').next()?.trim();
    validate_runtime_version(version).ok()?;
    Some(version.to_string())
}

fn validate_runtime_version(version: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !version.is_empty()
            && version.len() <= 80
            && version
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character)),
        "invalid package-manager version {version:?}"
    );
    Ok(())
}

fn host_home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| paths_equal(existing, &path)) {
        paths.push(path);
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    } else {
        left == right
    }
}

fn normalized_canonical_path(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let display = path.as_os_str().to_string_lossy();
        if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(native) = display.strip_prefix(r"\\?\") {
            return PathBuf::from(native);
        }
    }
    path
}

fn capsule_fingerprint(
    environment: &[(OsString, OsString)],
    path_entries: &[PathBuf],
    read_roots: &[PathBuf],
    managed_runtime_roots: &[PathBuf],
    issues: &[WorkspaceCapabilityIssue],
) -> String {
    let mut digest = Sha256::new();
    for (key, value) in environment {
        digest.update(key.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(value.to_string_lossy().as_bytes());
        digest.update([0xff]);
    }
    for path in path_entries {
        digest.update(b"path\0");
        digest.update(path.as_os_str().to_string_lossy().as_bytes());
        digest.update([0xff]);
    }
    for path in read_roots {
        digest.update(b"read\0");
        digest.update(path.as_os_str().to_string_lossy().as_bytes());
        digest.update([0xff]);
    }
    for path in managed_runtime_roots {
        digest.update(b"managed-runtime\0");
        digest.update(path.as_os_str().to_string_lossy().as_bytes());
        digest.update([0xff]);
    }
    for issue in issues {
        digest.update(issue.capability.as_bytes());
        digest.update([0]);
        digest.update(issue.reason.as_bytes());
        digest.update([0xff]);
    }
    let mut fingerprint = String::with_capacity(64);
    for byte in digest.finalize() {
        let _ = write!(&mut fingerprint, "{byte:02x}");
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("opentopia-capsule-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn write_fake_pnpm(root: &Path, version: &str) {
        fs::create_dir_all(root.join("bin")).expect("create fake pnpm");
        fs::write(
            root.join("package.json"),
            format!(
                r#"{{"name":"pnpm","version":"{version}","bin":{{"pnpm":"bin/pnpm.cjs","pnpx":"bin/pnpx.cjs"}}}}"#
            ),
        )
        .expect("write package metadata");
        fs::write(
            root.join("bin/pnpm.cjs"),
            r#"process.stdout.write("fake-pnpm")"#,
        )
        .expect("write pnpm bin");
        fs::write(
            root.join("bin/pnpx.cjs"),
            r#"process.stdout.write("fake-pnpx")"#,
        )
        .expect("write pnpx bin");
    }

    #[test]
    fn package_manager_declaration_is_exact_and_hash_suffix_is_ignored() {
        let root = fixture("declaration");
        fs::create_dir_all(&root).expect("create fixture");
        fs::write(
            root.join("package.json"),
            r#"{"packageManager":"pnpm@10.30.0+sha512.deadbeef"}"#,
        )
        .expect("write package json");
        assert_eq!(declared_pnpm_version(&root).as_deref(), Some("10.30.0"));
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn managed_pnpm_is_published_once_with_corepack_and_direct_shims() {
        let root = fixture("runtime");
        let source = root.join("source");
        let state = root.join("state");
        write_fake_pnpm(&source, "10.30.0");

        let first = prepare_managed_pnpm_at("10.30.0", &state, &[source.clone()])
            .expect("prepare managed pnpm");
        assert!(first.root.starts_with(&state));
        assert_eq!(first.permission_root, normalized_canonical_path(&state));
        assert!(first.corepack_home.join("v1/pnpm/10.30.0").is_dir());
        assert!(first.shims.is_dir());
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok()
        {
            #[cfg(windows)]
            let output = std::process::Command::new("cmd.exe")
                .args(["/d", "/c"])
                .arg(first.shims.join("pnpm.cmd"))
                .output()
                .expect("execute relocated pnpm shim");
            #[cfg(not(windows))]
            let output = std::process::Command::new(first.shims.join("pnpm"))
                .output()
                .expect("execute relocated pnpm shim");
            assert!(output.status.success());
            assert_eq!(String::from_utf8_lossy(&output.stdout), "fake-pnpm");
        }

        fs::remove_dir_all(source).expect("remove source");
        let second = prepare_managed_pnpm_at("10.30.0", &state, &[]).expect("reuse managed pnpm");
        assert_eq!(first.root, second.root);
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn capsule_git_policy_is_workspace_scoped() {
        let root = fixture("git");
        fs::create_dir_all(&root).expect("create fixture");
        let capsule = WorkspaceExecutionCapsule::discover(&root);
        let env = capsule
            .environment()
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(env.get("GIT_CONFIG_COUNT").map(String::as_str), Some("3"));
        assert_eq!(
            env.get("GIT_CONFIG_VALUE_0").map(PathBuf::from),
            Some(normalized_canonical_path(&root))
        );
        assert_eq!(capsule.fingerprint().len(), 64);
        fs::remove_dir_all(root).expect("remove fixture");
    }
}
