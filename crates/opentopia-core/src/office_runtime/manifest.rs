use super::runtime_lock::{OfficePythonAsset, OfficeRuntimeLock};
use serde::{Deserialize, Serialize};

pub(super) const OFFICE_RUNTIME_MANIFEST_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficeRuntimeManifest {
    pub(super) schema_version: u32,
    pub(super) id: String,
    pub(super) version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<String>,
    pub(super) python: OfficePythonManifest,
    pub(super) packages: OfficeRuntimePackages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficePythonManifest {
    pub(super) path: String,
    pub(super) sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) distribution: Option<OfficePythonDistributionManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficePythonDistributionManifest {
    pub(super) provider: String,
    pub(super) release: String,
    pub(super) target_triple: String,
    pub(super) archive_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OfficeRuntimePackages {
    pub(super) openpyxl: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) et_xmlfile: Option<String>,
}

pub(super) fn managed_manifest(
    lock: &OfficeRuntimeLock,
    target_id: &str,
    asset: &OfficePythonAsset,
    executable_sha256: String,
) -> OfficeRuntimeManifest {
    OfficeRuntimeManifest {
        schema_version: OFFICE_RUNTIME_MANIFEST_SCHEMA_VERSION,
        id: super::OFFICE_RUNTIME_ID.to_string(),
        version: lock.runtime_version.clone(),
        target: Some(target_id.to_string()),
        python: OfficePythonManifest {
            path: asset.python_path.clone(),
            sha256: executable_sha256,
            version: Some(lock.python_version.clone()),
            distribution: Some(OfficePythonDistributionManifest {
                provider: "astral-sh/python-build-standalone".to_string(),
                release: lock.python_release.clone(),
                target_triple: asset.target_triple.clone(),
                archive_sha256: asset.sha256.clone(),
            }),
        },
        packages: OfficeRuntimePackages {
            openpyxl: package_version(lock, "openpyxl"),
            et_xmlfile: Some(package_version(lock, "et_xmlfile")),
        },
    }
}

fn package_version(lock: &OfficeRuntimeLock, name: &str) -> String {
    lock.packages
        .iter()
        .find(|package| package.name == name)
        .map(|package| package.version.clone())
        .unwrap_or_default()
}
