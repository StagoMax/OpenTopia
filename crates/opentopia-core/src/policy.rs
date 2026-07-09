use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Chat,
    ReadOnly,
    Auto,
    Approve,
    FullAccess,
}

impl FromStr for PermissionMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "chat" => Ok(Self::Chat),
            "readonly" | "read_only" | "read-only" => Ok(Self::ReadOnly),
            "auto" => Ok(Self::Auto),
            "approve" => Ok(Self::Approve),
            "fullaccess" | "full_access" | "full-access" => Ok(Self::FullAccess),
            other => anyhow::bail!("unknown permission mode: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    Ask { reason: String },
}

pub trait PolicyEngine: Send + Sync {
    fn inspect_read(&self, path: &Path) -> PolicyDecision;
    fn inspect_write(&self, path: &Path) -> PolicyDecision;
    fn inspect_command(&self, command: &str) -> PolicyDecision;
}

#[derive(Debug, Clone)]
pub struct BasicPolicyEngine {
    workspace_root: PathBuf,
    mode: PermissionMode,
}

impl BasicPolicyEngine {
    pub fn new(workspace_root: PathBuf, mode: PermissionMode) -> Self {
        Self {
            workspace_root,
            mode,
        }
    }

    fn inside_workspace(&self, path: &Path) -> bool {
        if path.components().any(|component| matches!(component, Component::ParentDir)) {
            return false;
        }
        let workspace_root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            workspace_root.join(path)
        };
        let candidate = candidate.canonicalize().unwrap_or(candidate);
        candidate.starts_with(&workspace_root)
    }
}

impl PolicyEngine for BasicPolicyEngine {
    fn inspect_read(&self, path: &Path) -> PolicyDecision {
        if !self.inside_workspace(path) {
            return PolicyDecision::Ask {
                reason: format!("Reading outside the workspace: {}", path.display()),
            };
        }

        match self.mode {
            PermissionMode::Chat => PolicyDecision::Deny {
                reason: "Chat mode does not allow file access.".to_string(),
            },
            _ => PolicyDecision::Allow,
        }
    }

    fn inspect_write(&self, path: &Path) -> PolicyDecision {
        if !self.inside_workspace(path) {
            return PolicyDecision::Ask {
                reason: format!("Writing outside the workspace: {}", path.display()),
            };
        }

        match self.mode {
            PermissionMode::Chat | PermissionMode::ReadOnly => PolicyDecision::Deny {
                reason: "Current permission mode does not allow writes.".to_string(),
            },
            PermissionMode::Approve => PolicyDecision::Ask {
                reason: format!("Write requires approval: {}", path.display()),
            },
            PermissionMode::Auto | PermissionMode::FullAccess => PolicyDecision::Allow,
        }
    }

    fn inspect_command(&self, command: &str) -> PolicyDecision {
        let lowered = command.to_ascii_lowercase();
        let dangerous_markers = ["rm -rf", "del /s", "format ", "git reset --hard", "sudo "];
        if dangerous_markers
            .iter()
            .any(|marker| lowered.contains(marker))
        {
            if self.mode == PermissionMode::FullAccess {
                return PolicyDecision::Allow;
            }
            return PolicyDecision::Ask {
                reason: format!("Potentially destructive command: {command}"),
            };
        }

        match self.mode {
            PermissionMode::Chat | PermissionMode::ReadOnly => PolicyDecision::Deny {
                reason: "Current permission mode does not allow shell commands.".to_string(),
            },
            PermissionMode::Approve => PolicyDecision::Ask {
                reason: format!("Command requires approval: {command}"),
            },
            PermissionMode::Auto | PermissionMode::FullAccess => PolicyDecision::Allow,
        }
    }
}
