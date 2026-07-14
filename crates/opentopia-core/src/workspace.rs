use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: WorkspaceEntryKind,
    pub size: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTree {
    pub root: PathBuf,
    pub path: PathBuf,
    pub entries: Vec<WorkspaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilePreview {
    pub path: PathBuf,
    pub content: String,
    pub bytes: usize,
    pub truncated: bool,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: PathBuf,
    pub status: String,
    pub staged_status: String,
    pub unstaged_status: String,
    pub original_path: Option<PathBuf>,
    pub is_untracked: bool,
    pub is_renamed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDiffScope {
    Staged,
    Unstaged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiffHunk {
    pub path: PathBuf,
    pub scope: WorkspaceDiffScope,
    pub header: String,
    pub lines: Vec<String>,
    pub raw: String,
    pub patch: String,
    pub old_start: Option<u32>,
    pub old_lines: Option<u32>,
    pub new_start: Option<u32>,
    pub new_lines: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDiff {
    pub command: String,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub files: Vec<ChangedFile>,
    pub diff: String,
    pub staged_diff: String,
    pub unstaged_diff: String,
    pub hunks: Vec<WorkspaceDiffHunk>,
    pub truncated: bool,
    pub staged_truncated: bool,
    pub unstaged_truncated: bool,
}
