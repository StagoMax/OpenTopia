use crate::sandbox::{
    LocalSandboxConfig, NetworkPolicy, OsSandboxPlatform, SandboxBackendCapabilities, SandboxMode,
};
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
    /// The operation does not need network access. Enforce offline execution
    /// when the selected backend can do so authoritatively; otherwise retain
    /// the session's network boundary instead of rejecting a local process.
    PreferDeny,
    /// The operation requires authoritative offline execution. Unsupported
    /// backends must reject it rather than falling back to the session policy.
    Deny,
    InheritSession,
    Required,
}

impl NetworkAccess {
    pub fn does_not_require_network(self) -> bool {
        matches!(self, Self::PreferDeny | Self::Deny)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessLifetime {
    None,
    OneShot,
    Background,
    PersistentService,
}

/// How the authorization layer scopes extra per-call capabilities. Read paths
/// are projected without approval; write or host access still requires it.
/// Tools declare semantic needs and never manipulate platform ACLs directly.
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
            network: NetworkAccess::PreferDeny,
            process_lifetime: ProcessLifetime::None,
            approval_escalation: ApprovalEscalation::ExactPaths,
            requested_read_paths: read_paths.into_iter().collect(),
            requested_write_paths: Vec::new(),
        }
    }

    pub fn workspace_mutation(write_paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            filesystem: FilesystemAccess::WriteWorkspace,
            network: NetworkAccess::PreferDeny,
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
        let capabilities = SandboxBackendCapabilities::for_platform(
            OsSandboxPlatform::current(),
            base.effective_windows_backend(),
        );
        Self::resolve_with_capabilities(
            base,
            workspace_root,
            intent,
            approval_granted,
            &capabilities,
        )
    }

    fn resolve_with_capabilities(
        base: &LocalSandboxConfig,
        workspace_root: &Path,
        intent: &ToolExecutionIntent,
        approval_granted: bool,
        capabilities: &SandboxBackendCapabilities,
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
            NetworkAccess::PreferDeny if capabilities.network_offline => NetworkPolicy::Deny,
            NetworkAccess::PreferDeny => base.network,
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

        // A turn may carry exact path leases from earlier approvals. Project
        // only the leases needed by this declared intent into the call sandbox.
        // In particular, a later shell/process call must not inherit the parent
        // directory mount used to implement an earlier exact-file write.
        let leased_read_paths = read_paths
            .iter()
            .filter(|path| {
                base.is_within_approved_read_scope(path)
                    || base.is_within_approved_write_scope(path)
            })
            .cloned()
            .collect::<Vec<_>>();
        let leased_write_paths = write_paths
            .iter()
            .filter(|path| base.is_within_approved_write_scope(path))
            .cloned()
            .collect::<Vec<_>>();
        sandbox.approved_read_paths.clear();
        sandbox.approved_write_paths.clear();
        if intent.approval_escalation == ApprovalEscalation::ExactPaths {
            for path in leased_read_paths {
                sandbox.grant_read_path(path);
            }
            for path in leased_write_paths {
                sandbox.grant_write_path(path);
            }
        }

        match intent.approval_escalation {
            ApprovalEscalation::ExactPaths => {
                // Read access is part of both restricted sandbox profiles, not
                // an approval escalation. Project only the paths declared by
                // this call so Windows can provision normal-user ACL access
                // without widening later shell/process calls.
                for path in read_paths {
                    sandbox.grant_read_path(path);
                }
                if approval_granted || base.sandbox_mode == SandboxMode::DangerFullAccess {
                    for path in write_paths {
                        sandbox.grant_write_path(path);
                    }
                }
            }
            ApprovalEscalation::CommandScopedHostAccess if approval_granted => {
                // This grant is bound to one approved tool-call replay. It is
                // never persisted into the session profile or tool registry.
                sandbox = LocalSandboxConfig::danger_full_access();
            }
            ApprovalEscalation::None | ApprovalEscalation::CommandScopedHostAccess => {}
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

    fn capabilities_with_offline(network_offline: bool) -> SandboxBackendCapabilities {
        let mut capabilities = SandboxBackendCapabilities::for_platform(
            OsSandboxPlatform::Linux,
            crate::sandbox::WindowsSandboxBackend::Auto,
        );
        capabilities.network_offline = network_offline;
        capabilities
    }

    #[test]
    fn observation_enforces_offline_when_the_backend_supports_it() {
        let mut base = LocalSandboxConfig::enforce();
        base.network = NetworkPolicy::Allow;
        let grant = ExecutionGrant::resolve_with_capabilities(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::observation([]),
            false,
            &capabilities_with_offline(true),
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::ReadOnly);
        assert_eq!(grant.sandbox.network, NetworkPolicy::Deny);
    }

    #[test]
    fn observation_retains_session_network_when_offline_is_unsupported() {
        let mut base = LocalSandboxConfig::enforce();
        base.network = NetworkPolicy::Allow;
        let grant = ExecutionGrant::resolve_with_capabilities(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::observation([]),
            false,
            &capabilities_with_offline(false),
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::ReadOnly);
        assert_eq!(grant.sandbox.network, NetworkPolicy::Allow);
    }

    #[test]
    fn strict_offline_intent_is_never_downgraded() {
        let mut base = LocalSandboxConfig::enforce();
        base.network = NetworkPolicy::Allow;
        let mut intent = ToolExecutionIntent::observation([]);
        intent.network = NetworkAccess::Deny;
        let grant = ExecutionGrant::resolve_with_capabilities(
            &base,
            Path::new("C:/workspace"),
            &intent,
            false,
            &capabilities_with_offline(false),
        )
        .unwrap();

        assert_eq!(grant.sandbox.network, NetworkPolicy::Deny);
    }

    #[test]
    fn preferred_offline_does_not_widen_an_offline_session() {
        let mut base = LocalSandboxConfig::enforce();
        base.network = NetworkPolicy::Deny;
        let grant = ExecutionGrant::resolve_with_capabilities(
            &base,
            Path::new("C:/workspace"),
            &ToolExecutionIntent::workspace_mutation([]),
            false,
            &capabilities_with_offline(false),
        )
        .unwrap();

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
    fn observation_projects_only_the_declared_external_read_without_approval() {
        let id = uuid::Uuid::new_v4();
        let workspace = std::env::temp_dir().join(format!("opentopia-grant-workspace-{id}"));
        let outside = std::env::temp_dir().join(format!("opentopia-grant-outside-{id}"));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let requested = outside.join("requested.txt");
        let sibling = outside.join("sibling.txt");
        std::fs::write(&requested, "requested").unwrap();
        std::fs::write(&sibling, "sibling").unwrap();

        let grant = ExecutionGrant::resolve(
            &LocalSandboxConfig::enforce(),
            &workspace,
            &ToolExecutionIntent::observation([requested.clone()]),
            false,
        )
        .unwrap();

        assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::ReadOnly);
        assert!(grant.sandbox.is_within_approved_read_scope(&requested));
        assert!(!grant.sandbox.is_within_approved_read_scope(&sibling));
        assert!(grant.sandbox.approved_write_paths.is_empty());

        std::fs::remove_dir_all(workspace).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn unapproved_mutation_projects_reads_but_not_external_writes() {
        let base = LocalSandboxConfig::enforce();
        let read = PathBuf::from("C:/outside/input.txt");
        let write = PathBuf::from("C:/outside/output.txt");
        let intent = ToolExecutionIntent::workspace_mutation([write.clone()])
            .with_read_paths([read.clone()]);
        let grant =
            ExecutionGrant::resolve(&base, Path::new("C:/workspace"), &intent, false).unwrap();

        assert!(grant.sandbox.is_within_approved_read_scope(&read));
        assert!(!grant.sandbox.is_within_approved_write_scope(&write));
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

    #[test]
    fn exact_path_lease_is_projected_only_to_matching_declared_intent() {
        let workspace = Path::new("C:/workspace");
        let external = PathBuf::from("C:/outside/result.txt");
        let sibling = PathBuf::from("C:/outside/sibling.txt");
        let mut base = LocalSandboxConfig::enforce();
        base.grant_write_path(external.clone());

        let matching = ExecutionGrant::resolve(
            &base,
            workspace,
            &ToolExecutionIntent::workspace_mutation([external.clone()]),
            false,
        )
        .unwrap();
        assert!(matching.sandbox.is_within_approved_write_scope(&external));
        assert!(!matching.sandbox.is_within_approved_write_scope(&sibling));

        let command = ExecutionGrant::resolve(
            &base,
            workspace,
            &ToolExecutionIntent::session_process(ProcessLifetime::OneShot),
            false,
        )
        .unwrap();
        assert!(command.sandbox.approved_read_paths.is_empty());
        assert!(command.sandbox.approved_write_paths.is_empty());
    }
}
