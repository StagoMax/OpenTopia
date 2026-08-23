//! Filesystem scope normalization and protected-metadata policy.

use super::contract::{LocalSandboxConfig, OsSandboxMode, SandboxMode};
use std::path::{Path, PathBuf};

pub(super) fn path_is_within_approved_scope(path: &Path, approved: &Path) -> bool {
    let path = canonicalize_existing_ancestor(&absolute_path(path));
    let approved = canonicalize_existing_ancestor(&absolute_path(approved));
    let within_directory = if approved.is_dir() {
        #[cfg(windows)]
        {
            windows_path_starts_with(&path, &approved)
        }
        #[cfg(not(windows))]
        {
            path.starts_with(&approved)
        }
    } else {
        false
    };
    paths_equal(&path, &approved) || within_directory
}

pub(super) fn paths_equal(left: &Path, right: &Path) -> bool {
    let left = canonicalize_existing_ancestor(&absolute_path(left));
    let right = canonicalize_existing_ancestor(&absolute_path(right));
    #[cfg(windows)]
    {
        windows_comparison_path(&left).eq_ignore_ascii_case(&windows_comparison_path(&right))
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub(super) fn canonicalize_existing_ancestor(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let mut cursor = path;
    let mut missing = Vec::new();
    while let Some(parent) = cursor.parent() {
        if let Some(name) = cursor.file_name() {
            missing.push(name.to_os_string());
        }
        if let Ok(mut canonical) = parent.canonicalize() {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        cursor = parent;
    }
    path.to_path_buf()
}

pub(super) fn windows_comparison_path(path: &Path) -> String {
    let value = path_to_string(path).replace('/', "\\");
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        rest.to_string()
    } else if let Some(rest) = value.strip_prefix(r"\??\") {
        rest.to_string()
    } else {
        value
    }
}

pub(super) fn parse_enforcement_mode(value: &str) -> Option<OsSandboxMode> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "disabled" => Some(OsSandboxMode::Disabled),
        "best-effort" => Some(OsSandboxMode::BestEffort),
        "enforce" | "strict" => Some(OsSandboxMode::Enforce),
        _ => None,
    }
}

pub(super) fn env_path_list(name: &str) -> Vec<PathBuf> {
    std::env::var_os(name)
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

pub(super) fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(super) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(super) fn seatbelt_escape(path: &Path) -> String {
    path_to_string(path)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

const PROTECTED_METADATA_NAMES: [&str; 3] = [".git", ".agents", ".codex"];

pub fn is_protected_metadata_path(path: &Path, writable_root: &Path) -> bool {
    let candidate = absolute_path(path);
    let root = absolute_path(writable_root);
    let Ok(relative) = candidate.strip_prefix(root) else {
        return false;
    };
    relative.components().next().is_some_and(|component| {
        let name = component.as_os_str().to_string_lossy();
        PROTECTED_METADATA_NAMES
            .iter()
            .any(|protected| name.eq_ignore_ascii_case(protected))
    })
}

pub(super) fn protected_paths(workspace_root: &Path, config: &LocalSandboxConfig) -> Vec<PathBuf> {
    if config.sandbox_mode == SandboxMode::ReadOnly {
        // Dedicated Windows identities retain persistent account-level grants
        // from earlier workspace-write commands. A capability-scoped deny on
        // the current workspace keeps a later read-only launch read-only even
        // when its restricted token also carries the account SID for native
        // child-process IPC compatibility.
        return vec![absolute_path(workspace_root)];
    }
    if config.sandbox_mode == SandboxMode::DangerFullAccess {
        return Vec::new();
    }
    dedup_paths(
        config
            .effective_writable_roots(workspace_root)
            .into_iter()
            .flat_map(|root| {
                PROTECTED_METADATA_NAMES
                    .into_iter()
                    .map(move |name| root.join(name))
            }),
    )
}

pub(super) fn dedup_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths {
        let path = absolute_path(path);
        if !result.iter().any(|existing| existing == &path) {
            result.push(path);
        }
    }
    result
}

pub(super) fn windows_path_starts_with(path: &Path, root: &Path) -> bool {
    let path = canonicalize_existing_ancestor(&absolute_path(path));
    let root = canonicalize_existing_ancestor(&absolute_path(root));
    let path = windows_comparison_path(&path).to_ascii_lowercase();
    let root = windows_comparison_path(&root)
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}
