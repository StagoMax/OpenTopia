use super::OfficeRuntimeError;
use reqwest::Url;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const LOCK_CONTENT: &str = include_str!("../../../../runtime/office/runtime-lock.json");
const LOCK_ID: &str = "ai.opentopia.office-runtime.lock";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficeRuntimeLock {
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) runtime_version: String,
    pub(super) python_version: String,
    pub(super) python_release: String,
    pub(super) packages: Vec<OfficePackageAsset>,
    pub(super) targets: BTreeMap<String, OfficePythonAsset>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficePackageAsset {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) file_name: String,
    pub(super) url: String,
    pub(super) sha256: String,
    pub(super) max_bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficePythonAsset {
    pub(super) target_triple: String,
    pub(super) file_name: String,
    pub(super) url: String,
    pub(super) sha256: String,
    pub(super) max_bytes: u64,
    pub(super) python_path: String,
}

pub(super) fn runtime_lock() -> Result<&'static OfficeRuntimeLock, OfficeRuntimeError> {
    static LOCK: OnceLock<Result<OfficeRuntimeLock, String>> = OnceLock::new();
    LOCK.get_or_init(|| {
        serde_json::from_str::<OfficeRuntimeLock>(LOCK_CONTENT)
            .map_err(|error| format!("invalid embedded Office runtime lock: {error}"))
            .and_then(|lock| {
                validate_lock(&lock)?;
                Ok(lock)
            })
    })
    .as_ref()
    .map_err(|error| OfficeRuntimeError::Unavailable(error.clone()))
}

pub(super) fn current_target_id() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("windows", "aarch64") => Some("windows-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        _ => None,
    }
}

pub(super) fn current_python_asset() -> Result<
    (
        &'static OfficeRuntimeLock,
        &'static str,
        &'static OfficePythonAsset,
    ),
    OfficeRuntimeError,
> {
    let lock = runtime_lock()?;
    let target_id = current_target_id().ok_or_else(|| {
        OfficeRuntimeError::Unavailable(format!(
            "managed Office Python is unsupported on {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    let asset = lock.targets.get(target_id).ok_or_else(|| {
        OfficeRuntimeError::Unavailable(format!(
            "Office runtime lock has no Python asset for {target_id}"
        ))
    })?;
    Ok((lock, target_id, asset))
}

fn validate_lock(lock: &OfficeRuntimeLock) -> Result<(), String> {
    if lock.schema_version != 1 || lock.id != LOCK_ID {
        return Err("embedded Office runtime lock has an unsupported identity".to_string());
    }
    if lock.runtime_version.trim().is_empty()
        || lock.python_version.trim().is_empty()
        || lock.python_release.trim().is_empty()
    {
        return Err("embedded Office runtime lock has empty version metadata".to_string());
    }
    if lock.packages.is_empty() || lock.targets.is_empty() {
        return Err("embedded Office runtime lock has no artifacts".to_string());
    }
    for package in &lock.packages {
        validate_artifact(
            &package.file_name,
            &package.url,
            &package.sha256,
            package.max_bytes,
        )?;
        if package.name.trim().is_empty() || package.version.trim().is_empty() {
            return Err("Office package lock entry has empty identity metadata".to_string());
        }
    }
    if !lock
        .packages
        .iter()
        .any(|package| package.name == "openpyxl")
    {
        return Err("Office runtime lock does not pin openpyxl".to_string());
    }
    for (target_id, asset) in &lock.targets {
        validate_artifact(&asset.file_name, &asset.url, &asset.sha256, asset.max_bytes)?;
        if target_id.trim().is_empty()
            || asset.target_triple.trim().is_empty()
            || asset.python_path.trim().is_empty()
        {
            return Err("Office Python lock entry has empty target metadata".to_string());
        }
    }
    Ok(())
}

fn validate_artifact(
    file_name: &str,
    url: &str,
    sha256: &str,
    max_bytes: u64,
) -> Result<(), String> {
    if file_name.trim().is_empty() || file_name.contains(['/', '\\']) {
        return Err(format!("invalid locked artifact file name {file_name:?}"));
    }
    let url = Url::parse(url).map_err(|error| format!("invalid locked artifact URL: {error}"))?;
    if url.scheme() != "https" {
        return Err(format!("locked artifact URL must use HTTPS: {url}"));
    }
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "locked artifact {file_name} has an invalid SHA-256"
        ));
    }
    if max_bytes == 0 {
        return Err(format!("locked artifact {file_name} has no size limit"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_lock_is_valid_and_supports_the_current_target() {
        let lock = runtime_lock().expect("valid embedded Office runtime lock");
        assert_eq!(lock.python_version, "3.12.14");
        assert_eq!(lock.packages.len(), 2);
        let (_, target_id, asset) =
            current_python_asset().expect("current development target is supported");
        assert!(lock.targets.contains_key(target_id));
        assert_eq!(asset.sha256.len(), 64);
    }
}
