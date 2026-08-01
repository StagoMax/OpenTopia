//! Official plugins packaged with OpenTopia.
//!
//! Trust metadata lives in this host-owned registry rather than in plugin
//! manifests. Packages are materialized under a separate managed root so they
//! remain distinguishable from user-installed Codex-compatible plugins.

mod browser_automation;
mod computer_use;
mod spreadsheet;

use crate::plugins::PluginError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const RECEIPT_FILE: &str = ".opentopia-bundled.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundledPluginTrust {
    Standard,
    Official,
    Privileged,
    TrustedDriver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledPluginMetadata {
    pub name: &'static str,
    pub version: &'static str,
    pub trust: BundledPluginTrust,
    pub default_enabled: bool,
    pub native_capabilities: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BundledPluginFile {
    pub relative_path: &'static str,
    pub contents: &'static [u8],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BundledPluginPackage {
    pub metadata: BundledPluginMetadata,
    pub files: &'static [BundledPluginFile],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundledPluginInstallStatus {
    Installed,
    Updated,
    AlreadyCurrent,
    PreservedModified,
    PreservedUnmanaged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundledPluginInstallOutcome {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub status: BundledPluginInstallStatus,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundledPluginReceipt {
    plugin_name: String,
    installed_version: String,
    content_fingerprint: String,
}

const PACKAGES: &[BundledPluginPackage] = &[
    spreadsheet::PACKAGE,
    browser_automation::PACKAGE,
    computer_use::PACKAGE,
];

pub fn bundled_plugin_metadata(name: &str) -> Option<BundledPluginMetadata> {
    bundled_plugin_package(name).map(|package| package.metadata)
}

pub fn bundled_plugin_catalog() -> impl ExactSizeIterator<Item = BundledPluginMetadata> {
    PACKAGES.iter().map(|package| package.metadata)
}

pub(crate) fn verified_bundled_plugin_metadata(
    name: &str,
    plugin_root: &Path,
) -> Option<BundledPluginMetadata> {
    let package = bundled_plugin_package(name)?;
    let metadata = package.metadata;
    let packaged_fingerprint = package_fingerprint(package).ok()?;
    let receipt = read_receipt(plugin_root).ok()?;
    if receipt.plugin_name != name
        || receipt.installed_version != metadata.version
        || receipt.content_fingerprint != packaged_fingerprint
    {
        return None;
    }
    package_matches_directory(package, plugin_root)
        .ok()?
        .then_some(metadata)
}

fn bundled_plugin_package(name: &str) -> Option<&'static BundledPluginPackage> {
    PACKAGES
        .iter()
        .find(|package| package.metadata.name == name)
}

pub fn ensure_bundled_plugins_installed(
    destination_root: &Path,
) -> Result<Vec<BundledPluginInstallOutcome>, PluginError> {
    fs::create_dir_all(destination_root).map_err(io_error)?;
    let destination_root = destination_root.canonicalize().map_err(io_error)?;
    PACKAGES
        .iter()
        .map(|package| install_package(package, &destination_root))
        .collect()
}

fn install_package(
    package: &BundledPluginPackage,
    destination_root: &Path,
) -> Result<BundledPluginInstallOutcome, PluginError> {
    let destination = destination_root.join(package.metadata.name);
    let package_fingerprint = package_fingerprint(package)?;
    if destination.exists() {
        let receipt = match read_receipt(&destination) {
            Ok(receipt) if receipt.plugin_name == package.metadata.name => receipt,
            _ => {
                return Ok(outcome(
                    package,
                    destination,
                    BundledPluginInstallStatus::PreservedUnmanaged,
                ));
            }
        };
        let installed_fingerprint = directory_fingerprint(&destination)?;
        if installed_fingerprint != receipt.content_fingerprint {
            return Ok(outcome(
                package,
                destination,
                BundledPluginInstallStatus::PreservedModified,
            ));
        }
        if installed_fingerprint == package_fingerprint
            && receipt.installed_version == package.metadata.version
        {
            return Ok(outcome(
                package,
                destination,
                BundledPluginInstallStatus::AlreadyCurrent,
            ));
        }
        replace_package(
            package,
            destination_root,
            &destination,
            &package_fingerprint,
        )?;
        return Ok(outcome(
            package,
            destination,
            BundledPluginInstallStatus::Updated,
        ));
    }

    materialize_package(
        package,
        destination_root,
        &destination,
        &package_fingerprint,
    )?;
    Ok(outcome(
        package,
        destination,
        BundledPluginInstallStatus::Installed,
    ))
}

fn replace_package(
    package: &BundledPluginPackage,
    destination_root: &Path,
    destination: &Path,
    fingerprint: &str,
) -> Result<(), PluginError> {
    let staging = staging_path(destination_root, package.metadata.name);
    let backup = destination_root.join(format!(
        ".bundled-backup-{}-{}",
        package.metadata.name,
        Uuid::new_v4()
    ));
    write_package(package, &staging, fingerprint)?;
    fs::rename(destination, &backup).map_err(io_error)?;
    if let Err(error) = fs::rename(&staging, destination) {
        let _ = fs::rename(&backup, destination);
        let _ = fs::remove_dir_all(&staging);
        return Err(io_error(error));
    }
    let _ = fs::remove_dir_all(backup);
    Ok(())
}

fn materialize_package(
    package: &BundledPluginPackage,
    destination_root: &Path,
    destination: &Path,
    fingerprint: &str,
) -> Result<(), PluginError> {
    let staging = staging_path(destination_root, package.metadata.name);
    let result = (|| {
        write_package(package, &staging, fingerprint)?;
        fs::rename(&staging, destination).map_err(io_error)
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}

fn write_package(
    package: &BundledPluginPackage,
    destination: &Path,
    fingerprint: &str,
) -> Result<(), PluginError> {
    fs::create_dir(destination).map_err(io_error)?;
    for file in package.files {
        let relative = safe_relative_path(file.relative_path)?;
        let path = destination.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        fs::write(path, file.contents).map_err(io_error)?;
    }
    let receipt = BundledPluginReceipt {
        plugin_name: package.metadata.name.to_string(),
        installed_version: package.metadata.version.to_string(),
        content_fingerprint: fingerprint.to_string(),
    };
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| PluginError::InvalidManifest(error.to_string()))?;
    fs::write(destination.join(RECEIPT_FILE), bytes).map_err(io_error)
}

fn package_fingerprint(package: &BundledPluginPackage) -> Result<String, PluginError> {
    let files = package_files(package)?;
    Ok(fingerprint_files(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    ))
}

fn package_matches_directory(
    package: &BundledPluginPackage,
    root: &Path,
) -> Result<bool, PluginError> {
    let expected = package_files(package)?;
    let mut actual = BTreeMap::new();
    collect_files(root, root, &mut actual)?;
    Ok(actual == expected)
}

fn package_files(package: &BundledPluginPackage) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
    let mut files = BTreeMap::new();
    for file in package.files {
        let relative = normalized_relative_path(file.relative_path)?;
        if files.insert(relative, file.contents.to_vec()).is_some() {
            return Err(PluginError::InvalidManifest(format!(
                "bundled plugin {} contains duplicate files",
                package.metadata.name
            )));
        }
    }
    Ok(files)
}

fn directory_fingerprint(root: &Path) -> Result<String, PluginError> {
    let mut files = BTreeMap::new();
    collect_files(root, root, &mut files)?;
    Ok(fingerprint_files(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    ))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), PluginError> {
    for entry in fs::read_dir(directory).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let file_type = entry.file_type().map_err(io_error)?;
        if file_type.is_symlink() {
            return Err(PluginError::SymbolicLink(
                entry.path().display().to_string(),
            ));
        }
        if file_type.is_dir() {
            collect_files(root, &entry.path(), files)?;
        } else if file_type.is_file() && entry.file_name() != RECEIPT_FILE {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|error| PluginError::Io(error.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(relative, fs::read(entry.path()).map_err(io_error)?);
        }
    }
    Ok(())
}

fn fingerprint_files<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    // The receipt uses this only to detect whether an installed previous
    // version was edited before an upgrade. Official trust is established by
    // byte-for-byte comparison with the current embedded package instead.
    let mut hash = 0xcbf29ce484222325u64;
    for (path, contents) in files {
        for byte in path
            .as_bytes()
            .iter()
            .copied()
            .chain([0])
            .chain(contents.iter().copied())
            .chain([0xff])
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    format!("fnv1a64:{hash:016x}")
}

fn safe_relative_path(path: &str) -> Result<PathBuf, PluginError> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(PluginError::PathEscape(path.display().to_string()));
    }
    Ok(path.to_path_buf())
}

fn normalized_relative_path(path: &str) -> Result<String, PluginError> {
    Ok(safe_relative_path(path)?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn read_receipt(root: &Path) -> Result<BundledPluginReceipt, PluginError> {
    serde_json::from_slice(&fs::read(root.join(RECEIPT_FILE)).map_err(io_error)?)
        .map_err(|error| PluginError::InvalidManifest(error.to_string()))
}

fn staging_path(root: &Path, name: &str) -> PathBuf {
    root.join(format!(".bundled-installing-{name}-{}", Uuid::new_v4()))
}

fn outcome(
    package: &BundledPluginPackage,
    path: PathBuf,
    status: BundledPluginInstallStatus,
) -> BundledPluginInstallOutcome {
    BundledPluginInstallOutcome {
        name: package.metadata.name.to_string(),
        version: package.metadata.version.to_string(),
        path,
        status,
    }
}

fn io_error(error: std::io::Error) -> PluginError {
    PluginError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_V1: &[u8] = br#"{"name":"test-bundled","version":"1.0.0"}"#;
    const MANIFEST_V2: &[u8] = br#"{"name":"test-bundled","version":"2.0.0"}"#;
    const FILES_V1: &[BundledPluginFile] = &[BundledPluginFile {
        relative_path: ".codex-plugin/plugin.json",
        contents: MANIFEST_V1,
    }];
    const FILES_V2: &[BundledPluginFile] = &[BundledPluginFile {
        relative_path: ".codex-plugin/plugin.json",
        contents: MANIFEST_V2,
    }];

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("opentopia-bundled-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn package(version: &'static str, files: &'static [BundledPluginFile]) -> BundledPluginPackage {
        BundledPluginPackage {
            metadata: BundledPluginMetadata {
                name: "test-bundled",
                version,
                trust: BundledPluginTrust::Official,
                default_enabled: true,
                native_capabilities: &["test"],
            },
            files,
        }
    }

    #[test]
    fn install_is_idempotent_and_tracks_version() {
        let dir = TestDir::new();
        let package_v1 = package("1.0.0", FILES_V1);
        let first = install_package(&package_v1, &dir.0).unwrap();
        assert_eq!(first.status, BundledPluginInstallStatus::Installed);
        let second = install_package(&package_v1, &dir.0).unwrap();
        assert_eq!(second.status, BundledPluginInstallStatus::AlreadyCurrent);

        let package_v2 = package("2.0.0", FILES_V2);
        let upgraded = install_package(&package_v2, &dir.0).unwrap();
        assert_eq!(upgraded.status, BundledPluginInstallStatus::Updated);
        let receipt = read_receipt(&dir.0.join("test-bundled")).unwrap();
        assert_eq!(receipt.installed_version, "2.0.0");
        assert_eq!(
            fs::read(dir.0.join("test-bundled/.codex-plugin/plugin.json")).unwrap(),
            MANIFEST_V2
        );
    }

    #[test]
    fn modified_installation_is_never_overwritten() {
        let dir = TestDir::new();
        let package_v1 = package("1.0.0", FILES_V1);
        install_package(&package_v1, &dir.0).unwrap();
        let manifest = dir.0.join("test-bundled/.codex-plugin/plugin.json");
        fs::write(&manifest, b"user change").unwrap();

        let result = install_package(&package("2.0.0", FILES_V2), &dir.0).unwrap();
        assert_eq!(result.status, BundledPluginInstallStatus::PreservedModified);
        assert_eq!(fs::read(manifest).unwrap(), b"user change");
    }

    #[test]
    fn unmanaged_same_name_directory_is_never_overwritten() {
        let dir = TestDir::new();
        let target = dir.0.join("test-bundled");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("user.txt"), b"keep").unwrap();

        let result = install_package(&package("1.0.0", FILES_V1), &dir.0).unwrap();
        assert_eq!(
            result.status,
            BundledPluginInstallStatus::PreservedUnmanaged
        );
        assert_eq!(fs::read(target.join("user.txt")).unwrap(), b"keep");
    }

    #[test]
    fn edited_package_cannot_restore_official_trust_by_rewriting_receipt() {
        let dir = TestDir::new();
        ensure_bundled_plugins_installed(&dir.0).unwrap();
        let plugin_root = dir.0.join("computer-use");
        fs::write(
            plugin_root.join("configuration.schema.json"),
            b"user-modified schema",
        )
        .unwrap();
        let mut receipt = read_receipt(&plugin_root).unwrap();
        receipt.content_fingerprint = directory_fingerprint(&plugin_root).unwrap();
        fs::write(
            plugin_root.join(RECEIPT_FILE),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();

        assert!(verified_bundled_plugin_metadata("computer-use", &plugin_root).is_none());
    }
}
