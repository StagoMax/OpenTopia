use super::manifest::managed_manifest;
use super::runtime_lock::{current_python_asset, OfficePackageAsset};
use super::{resolve_runtime_at, OfficePythonRuntime, OfficeRuntimeSource};
use crate::managed_runtime_download::{
    file_sha256, managed_runtime_root, ManagedRuntimeArtifact, ManagedRuntimeDownloadPolicy,
    ManagedRuntimeDownloader, VerifiedRuntimeDownload,
};
use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use futures_util::future::try_join_all;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tar::Archive;
use tracing::warn;
use uuid::Uuid;

const MAX_ARCHIVE_ENTRIES: usize = 30_000;
const MAX_EXTRACTED_BYTES: u64 = 768 * 1024 * 1024;
const PACKAGE_INSTALL_TIMEOUT: Duration = Duration::from_secs(3 * 60);

pub(super) fn managed_office_runtime_directory() -> Result<PathBuf> {
    let (lock, target_id, _) = current_python_asset().map_err(anyhow::Error::new)?;
    Ok(managed_runtime_root()
        .join("office-python")
        .join(&lock.runtime_version)
        .join(target_id))
}

pub(super) async fn ensure_managed_office_runtime() -> Result<OfficePythonRuntime> {
    let (lock, target_id, python_asset) = current_python_asset().map_err(anyhow::Error::new)?;
    let final_directory = managed_office_runtime_directory()?;
    if let Ok(runtime) = resolve_runtime_at(&final_directory, OfficeRuntimeSource::Managed) {
        return Ok(runtime);
    }

    let runtime_root = managed_runtime_root();
    let download_directory = runtime_root.join(".downloads");
    let downloader = ManagedRuntimeDownloader::new(
        "OpenTopia managed Office runtime installer",
        ManagedRuntimeDownloadPolicy::default(),
    )?;
    let mut artifacts = Vec::with_capacity(1 + lock.packages.len());
    artifacts.push(ManagedRuntimeArtifact {
        url: python_asset
            .url
            .parse()
            .context("locked Office Python URL is invalid")?,
        file_name: python_asset.file_name.clone(),
        expected_sha256: python_asset.sha256.clone(),
        download_directory: download_directory.clone(),
        max_bytes: python_asset.max_bytes,
    });
    for package in &lock.packages {
        artifacts.push(package_download(package, &download_directory)?);
    }

    // Independent artifacts download concurrently. The downloader owns each
    // partial file from creation, so cancellation caused by a sibling failure
    // also removes the unfinished artifact.
    let downloads = try_join_all(
        artifacts
            .iter()
            .map(|artifact| downloader.download_verified(artifact)),
    )
    .await?;
    let (python_download, package_downloads) = downloads
        .split_first()
        .context("Office runtime download set was empty")?;

    let staging_parent = final_directory
        .parent()
        .context("managed Office runtime has no parent directory")?;
    tokio::fs::create_dir_all(staging_parent).await?;
    let mut staging = StagingDirectory::create(staging_parent.join(format!(
        ".office-python-{}-{}",
        lock.runtime_version,
        Uuid::new_v4()
    )))?;
    let archive_path = python_download.path().to_path_buf();
    let staging_path = staging.path().to_path_buf();
    tokio::task::spawn_blocking(move || extract_python_archive(&archive_path, &staging_path))
        .await
        .context("Office Python extraction task failed")??;

    let python = staging.path().join(&python_asset.python_path);
    anyhow::ensure!(
        python.is_file(),
        "standalone Python archive did not contain {}",
        python_asset.python_path
    );
    install_packages(
        &python,
        staging.path(),
        target_id,
        &lock.python_version,
        package_downloads,
        &lock.packages,
    )
    .await?;

    let executable_sha256 = file_sha256(&python).await?;
    let manifest = managed_manifest(lock, target_id, python_asset, executable_sha256);
    let mut encoded = serde_json::to_string_pretty(&manifest)?;
    encoded.push('\n');
    tokio::fs::write(staging.path().join(super::OFFICE_RUNTIME_MANIFEST), encoded).await?;
    resolve_runtime_at(staging.path(), OfficeRuntimeSource::Managed)
        .context("prepared Office runtime failed its staged probe")?;

    if final_directory.exists()
        && resolve_runtime_at(&final_directory, OfficeRuntimeSource::Managed).is_err()
    {
        tokio::fs::remove_dir_all(&final_directory)
            .await
            .context("failed to replace an invalid managed Office runtime")?;
    }
    match tokio::fs::rename(staging.path(), &final_directory).await {
        Ok(()) => staging.disarm(),
        Err(error)
            if resolve_runtime_at(&final_directory, OfficeRuntimeSource::Managed).is_ok() =>
        {
            warn!(%error, "another process installed the managed Office runtime first");
        }
        Err(error) => {
            return Err(error).context("failed to activate managed Office runtime");
        }
    }
    resolve_runtime_at(&final_directory, OfficeRuntimeSource::Managed)
        .context("activated managed Office runtime failed its final probe")
}

fn package_download(
    package: &OfficePackageAsset,
    directory: &Path,
) -> Result<ManagedRuntimeArtifact> {
    Ok(ManagedRuntimeArtifact {
        url: package
            .url
            .parse()
            .with_context(|| format!("locked {} package URL is invalid", package.name))?,
        file_name: package.file_name.clone(),
        expected_sha256: package.sha256.clone(),
        download_directory: directory.to_path_buf(),
        max_bytes: package.max_bytes,
    })
}

async fn install_packages(
    python: &Path,
    runtime_root: &Path,
    target_id: &str,
    python_version: &str,
    downloads: &[VerifiedRuntimeDownload],
    packages: &[OfficePackageAsset],
) -> Result<()> {
    anyhow::ensure!(
        downloads.len() == packages.len(),
        "Office package download set does not match the runtime lock"
    );
    let purelib = python_site_packages(runtime_root, target_id, python_version)?;
    tokio::fs::create_dir_all(&purelib).await?;
    let wheel_directory = runtime_root.join(".wheels");
    tokio::fs::create_dir_all(&wheel_directory).await?;
    let mut wheel_paths = Vec::with_capacity(packages.len());
    for (download, package) in downloads.iter().zip(packages) {
        let wheel = wheel_directory.join(&package.file_name);
        tokio::fs::copy(download.path(), &wheel).await?;
        wheel_paths.push(wheel);
    }
    let mut command = tokio::process::Command::new(python);
    command
        .args([
            "-I",
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--no-input",
            "--no-index",
            "--no-deps",
            "--no-compile",
            "--target",
        ])
        .arg(&purelib);
    for wheel in &wheel_paths {
        command.arg(wheel);
    }
    command
        .current_dir(runtime_root)
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PACKAGE_INSTALL_TIMEOUT, command.output())
        .await
        .context("installing Office Python packages timed out")??;
    let result = if output.status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "installing verified Office Python packages failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    };
    let cleanup = tokio::fs::remove_dir_all(&wheel_directory).await;
    result?;
    cleanup.context("failed to clean staged Office package wheels")
}

fn python_site_packages(
    runtime_root: &Path,
    target_id: &str,
    python_version: &str,
) -> Result<PathBuf> {
    if target_id.starts_with("windows-") {
        return Ok(runtime_root
            .join("python")
            .join("Lib")
            .join("site-packages"));
    }
    let mut components = python_version.split('.');
    let major = components.next().context("Python version has no major")?;
    let minor = components.next().context("Python version has no minor")?;
    Ok(runtime_root
        .join("python")
        .join("lib")
        .join(format!("python{major}.{minor}"))
        .join("site-packages"))
}

fn extract_python_archive(archive_path: &Path, output_root: &Path) -> Result<()> {
    fs::create_dir_all(output_root)?;
    let file = fs::File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut entries = 0_usize;
    let mut extracted = 0_u64;
    for entry in archive
        .entries()
        .context("invalid standalone Python archive")?
    {
        let mut entry = entry?;
        entries = entries.saturating_add(1);
        anyhow::ensure!(
            entries <= MAX_ARCHIVE_ENTRIES,
            "standalone Python archive contains too many entries"
        );
        extracted = extracted.saturating_add(entry.size());
        anyhow::ensure!(
            extracted <= MAX_EXTRACTED_BYTES,
            "standalone Python archive exceeds the extraction limit"
        );
        let path = entry.path()?.into_owned();
        anyhow::ensure!(
            safe_python_archive_path(&path),
            "unsafe Python archive path {path:?}"
        );
        let entry_type = entry.header().entry_type();
        anyhow::ensure!(
            entry_type.is_file()
                || entry_type.is_dir()
                || entry_type.is_symlink()
                || entry_type.is_hard_link(),
            "unsupported Python archive entry type for {path:?}"
        );
        anyhow::ensure!(
            entry.unpack_in(output_root)?,
            "Python archive entry escaped the staging directory: {path:?}"
        );
    }
    Ok(())
}

fn safe_python_archive_path(path: &Path) -> bool {
    let mut components = path.components();
    matches!(components.next(), Some(Component::Normal(root)) if root == "python")
        && components.all(|component| matches!(component, Component::Normal(_)))
}

struct StagingDirectory {
    path: PathBuf,
    remove_on_drop: bool,
}

impl StagingDirectory {
    fn create(path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            remove_on_drop: true,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.remove_on_drop = false;
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.remove_on_drop {
            if let Err(error) = fs::remove_dir_all(&self.path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!(
                        path = %self.path.display(),
                        %error,
                        "failed to clean managed Office runtime staging directory"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_paths_must_stay_under_python_root() {
        assert!(safe_python_archive_path(Path::new("python/Lib/site.py")));
        assert!(!safe_python_archive_path(Path::new("../python.exe")));
        assert!(!safe_python_archive_path(Path::new("python/../outside")));
        assert!(!safe_python_archive_path(Path::new("other/python.exe")));
    }

    #[test]
    fn site_packages_path_matches_supported_layouts() {
        let root = Path::new("runtime");
        assert_eq!(
            python_site_packages(root, "windows-x86_64", "3.12.14").unwrap(),
            root.join("python").join("Lib").join("site-packages")
        );
        assert_eq!(
            python_site_packages(root, "linux-x86_64", "3.12.14").unwrap(),
            root.join("python")
                .join("lib")
                .join("python3.12")
                .join("site-packages")
        );
    }

    #[tokio::test]
    #[ignore = "downloads the pinned standalone Python distribution"]
    async fn managed_installer_downloads_verifies_and_activates_the_runtime() {
        let runtime = ensure_managed_office_runtime()
            .await
            .expect("managed Office runtime installation must succeed");
        assert_eq!(runtime.python_version, "3.12.14");
        assert_eq!(runtime.openpyxl_version, "3.1.5");
        assert_eq!(runtime.source, OfficeRuntimeSource::Managed);
        assert!(runtime.executable.starts_with(runtime.root));
    }
}
