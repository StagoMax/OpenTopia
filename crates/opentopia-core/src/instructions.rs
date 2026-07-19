use crate::model_context::{content_fingerprint, InstructionSnapshotRef};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_INSTRUCTION_FILES: usize = 16;
const MAX_INSTRUCTION_BYTES: usize = 64 * 1024;
const MAX_TOTAL_INSTRUCTION_BYTES: usize = 192 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstructionScope {
    User,
    Workspace,
    Nested,
}

impl InstructionScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Workspace => "workspace",
            Self::Nested => "nested",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDocument {
    pub scope: InstructionScope,
    pub path: PathBuf,
    pub content: String,
    pub content_hash: String,
    pub bytes: usize,
    pub truncated: bool,
}

impl InstructionDocument {
    pub fn snapshot_ref(&self) -> InstructionSnapshotRef {
        InstructionSnapshotRef {
            scope: self.scope.as_str().to_string(),
            path: self.path.clone(),
            content_hash: self.content_hash.clone(),
            bytes: self.bytes,
            truncated: self.truncated,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionResolution {
    pub documents: Vec<InstructionDocument>,
    pub warnings: Vec<String>,
}

pub fn resolve_instruction_documents(workspace_root: &Path, cwd: &Path) -> InstructionResolution {
    let mut resolution = InstructionResolution::default();
    let mut remaining = MAX_TOTAL_INSTRUCTION_BYTES;
    let mut seen = HashSet::new();

    for path in user_instruction_candidates() {
        load_candidate(
            &path,
            InstructionScope::User,
            &mut remaining,
            &mut seen,
            &mut resolution,
        );
    }

    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let effective_cwd = if cwd.starts_with(&root) {
        cwd
    } else {
        root.clone()
    };
    for (index, directory) in directory_chain(&root, &effective_cwd)
        .into_iter()
        .enumerate()
    {
        let scope = if index == 0 {
            InstructionScope::Workspace
        } else {
            InstructionScope::Nested
        };
        let override_path = directory.join("AGENTS.override.md");
        let path = if override_path.is_file() {
            override_path
        } else {
            directory.join("AGENTS.md")
        };
        load_candidate(&path, scope, &mut remaining, &mut seen, &mut resolution);
    }

    resolution
}

fn load_candidate(
    path: &Path,
    scope: InstructionScope,
    remaining: &mut usize,
    seen: &mut HashSet<PathBuf>,
    resolution: &mut InstructionResolution,
) {
    if resolution.documents.len() >= MAX_INSTRUCTION_FILES || *remaining == 0 {
        return;
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        resolution.warnings.push(format!(
            "ignored non-regular instruction file: {}",
            path.display()
        ));
        return;
    }
    let Ok(canonical) = path.canonicalize() else {
        resolution.warnings.push(format!(
            "could not resolve instruction file: {}",
            path.display()
        ));
        return;
    };
    if !seen.insert(canonical.clone()) {
        return;
    }
    let Ok(bytes) = fs::read(&canonical) else {
        resolution.warnings.push(format!(
            "could not read instruction file: {}",
            canonical.display()
        ));
        return;
    };
    let limit = MAX_INSTRUCTION_BYTES.min(*remaining);
    let truncated = bytes.len() > limit;
    let selected = &bytes[..bytes.len().min(limit)];
    let content = String::from_utf8_lossy(selected).into_owned();
    *remaining = remaining.saturating_sub(selected.len());
    resolution.documents.push(InstructionDocument {
        scope,
        path: canonical,
        content_hash: content_fingerprint(selected),
        bytes: bytes.len(),
        content,
        truncated,
    });
}

fn user_instruction_candidates() -> Vec<PathBuf> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    vec![
        home.join(".codex").join("AGENTS.md"),
        home.join(".opentopia").join("AGENTS.md"),
    ]
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn directory_chain(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut chain = vec![root.to_path_buf()];
    let Ok(relative) = cwd.strip_prefix(root) else {
        return chain;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        chain.push(current.clone());
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("opentopia-{label}-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn nested_instructions_follow_root_to_cwd_order() {
        let root = temp_workspace("instructions");
        let nested = root.join("crates").join("core");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("AGENTS.md"), "root rules").unwrap();
        fs::write(root.join("crates").join("AGENTS.md"), "crate rules").unwrap();

        let resolution = resolve_instruction_documents(&root, &nested);
        let canonical_root = root.canonicalize().unwrap();
        let workspace_documents = resolution
            .documents
            .iter()
            .filter(|document| document.path.starts_with(&canonical_root))
            .collect::<Vec<_>>();
        assert_eq!(workspace_documents.len(), 2);
        assert_eq!(workspace_documents[0].content, "root rules");
        assert_eq!(workspace_documents[1].content, "crate rules");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn override_file_wins_within_one_directory() {
        let root = temp_workspace("instruction-override");
        fs::write(root.join("AGENTS.md"), "normal").unwrap();
        fs::write(root.join("AGENTS.override.md"), "override").unwrap();

        let resolution = resolve_instruction_documents(&root, &root);
        let canonical_root = root.canonicalize().unwrap();
        let document = resolution
            .documents
            .iter()
            .find(|document| document.path.starts_with(&canonical_root))
            .unwrap();
        assert_eq!(document.content, "override");

        fs::remove_dir_all(root).unwrap();
    }
}
