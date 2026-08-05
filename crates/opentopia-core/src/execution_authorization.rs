use crate::sandbox::{LocalSandboxConfig, NetworkPolicy, SandboxMode};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Filesystem authority requested by one tool call. This is an intent, not an
/// authorization decision: the active session profile remains the upper bound
/// unless the user explicitly approves a scoped escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    None,
    ReadWorkspace,
    WriteWorkspace,
    InheritSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkAccess {
    Deny,
    InheritSession,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessLifetime {
    None,
    OneShot,
    Background,
    PersistentService,
}

/// How a user approval may widen the active session profile for this call.
/// Tools declare semantic escalation needs; they never manipulate platform ACLs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalEscalation {
    None,
    ExactPaths,
    CommandScopedHostAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionIntent {
    pub filesystem: FilesystemAccess,
    pub network: NetworkAccess,
    pub process_lifetime: ProcessLifetime,
    pub approval_escalation: ApprovalEscalation,
    pub requested_read_paths: Vec<PathBuf>,
    pub requested_write_paths: Vec<PathBuf>,
}

impl ToolExecutionIntent {
    pub fn observation(read_paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            filesystem: FilesystemAccess::ReadWorkspace,
            network: NetworkAccess::Deny,
            process_lifetime: ProcessLifetime::None,
            approval_escalation: ApprovalEscalation::ExactPaths,
            requested_read_paths: read_paths.into_iter().collect(),
            requested_write_paths: Vec::new(),
        }
    }

    pub fn workspace_mutation(write_paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            filesystem: FilesystemAccess::WriteWorkspace,
            network: NetworkAccess::Deny,
            process_lifetime: ProcessLifetime::None,
            approval_escalation: ApprovalEscalation::ExactPaths,
            requested_read_paths: Vec::new(),
            requested_write_paths: write_paths.into_iter().collect(),
        }
    }

    pub fn session_process(lifetime: ProcessLifetime) -> Self {
        Self {
            filesystem: FilesystemAccess::InheritSession,
            network: NetworkAccess::InheritSession,
            process_lifetime: lifetime,
            approval_escalation: ApprovalEscalation::CommandScopedHostAccess,
            requested_read_paths: Vec::new(),
            requested_write_paths: Vec::new(),
        }
    }

    pub fn external() -> Self {
        Self {
            filesystem: FilesystemAccess::None,
            network: NetworkAccess::InheritSession,
            process_lifetime: ProcessLifetime::None,
            approval_escalation: ApprovalEscalation::None,
            requested_read_paths: Vec::new(),
            requested_write_paths: Vec::new(),
        }
    }

    pub fn with_process_lifetime(mut self, lifetime: ProcessLifetime) -> Self {
        self.process_lifetime = lifetime;
        self
    }

    pub fn with_read_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.requested_read_paths.extend(paths);
        self
    }

    pub fn with_write_paths(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.requested_write_paths.extend(paths);
        self
    }
}

impl Default for ToolExecutionIntent {
    fn default() -> Self {
        Self {
            filesystem: FilesystemAccess::InheritSession,
            network: NetworkAccess::InheritSession,
            process_lifetime: ProcessLifetime::None,
            approval_escalation: ApprovalEscalation::None,
            requested_read_paths: Vec::new(),
            requested_write_paths: Vec::new(),
        }
    }
}

/// Effective, platform-neutral authority for one tool call. The local execution
/// backend maps its sandbox profile to Windows, Linux, or macOS mechanisms.
#[derive(Debug, Clone)]
pub struct ExecutionGrant {
    pub sandbox: LocalSandboxConfig,
    pub process_lifetime: ProcessLifetime,
}

impl ExecutionGrant {
    pub fn resolve(
        base: &LocalSandboxConfig,
        workspace_root: &Path,
        intent: &ToolExecutionIntent,
        approval_granted: bool,
    ) -> anyhow::Result<Self> {
        let mut sandbox = base.clone();

        sandbox = match intent.filesystem {
            FilesystemAccess::None | FilesystemAccess::ReadWorkspace => {
                sandbox.with_sandbox_mode(SandboxMode::ReadOnly)
            }
            FilesystemAccess::WriteWorkspace => match base.sandbox_mode {
                SandboxMode::ReadOnly => sandbox.with_sandbox_mode(SandboxMode::ReadOnly),
                SandboxMode::WorkspaceWrite | SandboxMode::DangerFullAccess => {
                    sandbox.with_sandbox_mode(SandboxMode::WorkspaceWrite)
                }
            },
            FilesystemAccess::InheritSession => sandbox,
        };

        sandbox.network = match intent.network {
            NetworkAccess::Deny => NetworkPolicy::Deny,
            NetworkAccess::InheritSession => base.network,
            NetworkAccess::Required if approval_granted => NetworkPolicy::Allow,
            NetworkAccess::Required => base.network,
        };

        let read_paths = intent
            .requested_read_paths
            .iter()
            .map(|path| resolve_requested_path(workspace_root, path))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let write_paths = intent
            .requested_write_paths
            .iter()
            .map(|path| resolve_requested_path(workspace_root, path))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let exact_paths_are_authorized =
            approval_granted || base.sandbox_mode == SandboxMode::DangerFullAccess;
        match intent.approval_escalation {
            ApprovalEscalation::ExactPaths if exact_paths_are_authorized => {
                for path in read_paths {
                    sandbox.grant_read_path(path);
                }
                for path in write_paths {
                    sandbox.grant_write_path(path);
                }
            }
            ApprovalEscalation::CommandScopedHostAccess if approval_granted => {
                // This grant is bound to one approved tool-call replay. It is
                // never persisted into the session profile or tool registry.
                sandbox = LocalSandboxConfig::danger_full_access();
            }
            ApprovalEscalation::None
            | ApprovalEscalation::ExactPaths
            | ApprovalEscalation::CommandScopedHostAccess => {}
        }

        Ok(Self {
            sandbox,
            process_lifetime: intent.process_lifetime,
        })
    }
}

fn resolve_requested_path(workspace_root: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !path
            .components()
            .any(|component| matches!(component, Component::ParentDir)),
        "authorized path cannot contain '..': {}",
        path.display()
    );
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_narrows_workspace_write_and_network() {
        let mut base = LocalSandboxConfig::enforce();
        base.network = NetworkPolicy::Allow;
        let grant = ExecutionGrant::resolve(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::observation([]),
            false,
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::ReadOnly);
        assert_eq!(grant.sandbox.network, NetworkPolicy::Deny);
    }

    #[test]
    fn workspace_mutation_never_widens_read_only_session() {
        let base = LocalSandboxConfig::enforce().with_sandbox_mode(SandboxMode::ReadOnly);
        let grant = ExecutionGrant::resolve(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::workspace_mutation([PathBuf::from("src/lib.rs")]),
            false,
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::ReadOnly);
    }

    #[test]
    fn approval_adds_only_declared_exact_path() {
        let base = LocalSandboxConfig::enforce();
        let requested = PathBuf::from("C:/outside/result.txt");
        let grant = ExecutionGrant::resolve(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::workspace_mutation([requested.clone()]),
            true,
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert!(grant.sandbox.is_approved_write_path(&requested));
        assert_ne!(grant.sandbox.sandbox_mode, SandboxMode::DangerFullAccess);
    }

    #[test]
    fn approved_command_escalation_is_explicitly_call_scoped_full_access() {
        let base = LocalSandboxConfig::enforce();
        let intent = ToolExecutionIntent::session_process(ProcessLifetime::OneShot);
        let ordinary =
            ExecutionGrant::resolve(&base, Path::new("C:/workspace"), &intent, false).unwrap();
        let approved =
            ExecutionGrant::resolve(&base, Path::new("C:/workspace"), &intent, true).unwrap();

        assert_eq!(ordinary.sandbox.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert_eq!(approved.sandbox.sandbox_mode, SandboxMode::DangerFullAccess);
    }

    #[test]
    fn full_access_session_keeps_exact_external_path_while_narrowing_tool_mode() {
        let base = LocalSandboxConfig::danger_full_access();
        let external = PathBuf::from("C:/outside/result.txt");
        let grant = ExecutionGrant::resolve(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::workspace_mutation([external.clone()]),
            false,
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert!(grant.sandbox.is_approved_write_path(&external));
    }
}
