use crate::enterprise::CapabilityProjection;
use crate::policy::{BasicPolicyEngine, PermissionMode};
use crate::sandbox::LocalSandboxConfig;
use crate::tools::ToolInvocationContext;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Immutable, validated authority for one Agent run.
///
/// Permission mode, sandboxing, workspace identity, and capability projection
/// are kept together so callers cannot accidentally combine facts resolved for
/// different turns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionAuthority {
    workspace_root: PathBuf,
    permission_mode: PermissionMode,
    sandbox_config: LocalSandboxConfig,
    capability_projection: CapabilityProjection,
}

impl ExecutionAuthority {
    pub fn new(
        workspace_root: PathBuf,
        permission_mode: PermissionMode,
        sandbox_config: LocalSandboxConfig,
        capability_projection: CapabilityProjection,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            capability_projection.allows_workspace_root(&workspace_root),
            "workspace root is outside the execution authority projection: {}",
            workspace_root.display()
        );
        anyhow::ensure!(
            !workspace_root.as_os_str().is_empty(),
            "execution authority requires a workspace root"
        );
        Ok(Self {
            workspace_root,
            permission_mode,
            sandbox_config,
            capability_projection,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn permission_mode(&self) -> PermissionMode {
        self.permission_mode
    }

    pub fn sandbox_config(&self) -> &LocalSandboxConfig {
        &self.sandbox_config
    }

    pub fn capability_projection(&self) -> &CapabilityProjection {
        &self.capability_projection
    }

    pub fn validate_workspace(&self, workspace_root: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            same_workspace(&self.workspace_root, workspace_root),
            "turn workspace does not match its execution authority: expected {}, found {}",
            self.workspace_root.display(),
            workspace_root.display()
        );
        Ok(())
    }

    /// Creates a tool context whose policy, sandbox, permission mode, and
    /// capability projection all come from this one authority snapshot.
    pub fn local_tool_context(&self) -> ToolInvocationContext {
        let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
            self.workspace_root.clone(),
            self.permission_mode,
            &self.sandbox_config,
        ));
        ToolInvocationContext::local_with_authority(
            self.workspace_root.clone(),
            policy,
            self.permission_mode,
            self.sandbox_config.clone(),
            self.capability_projection.clone(),
        )
    }

    pub(crate) fn with_projection(&self, projection: CapabilityProjection) -> anyhow::Result<Self> {
        Self::new(
            self.workspace_root.clone(),
            self.permission_mode,
            self.sandbox_config.clone(),
            projection,
        )
    }

    pub(crate) fn with_sandbox(&self, sandbox_config: LocalSandboxConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(
            sandbox_config
                .sandbox_mode
                .is_attenuation_of(self.sandbox_config.sandbox_mode),
            "sandbox authority cannot be widened while preparing a run"
        );
        anyhow::ensure!(
            sandbox_config
                == self
                    .sandbox_config
                    .clone()
                    .with_sandbox_mode(sandbox_config.sandbox_mode),
            "sandbox attenuation may only change the policy-level sandbox mode"
        );
        Self::new(
            self.workspace_root.clone(),
            self.permission_mode,
            sandbox_config,
            self.capability_projection.clone(),
        )
    }
}

fn same_workspace(left: &Path, right: &Path) -> bool {
    let normalized = |path: &Path| {
        path.canonicalize()
            .with_context(|| format!("resolve workspace {}", path.display()))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    normalized(left) == normalized(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_rejects_a_workspace_outside_the_projection() {
        let error = ExecutionAuthority::new(
            PathBuf::from("workspace"),
            PermissionMode::Auto,
            LocalSandboxConfig::default(),
            CapabilityProjection::deny_all(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("outside"));
    }

    #[test]
    fn tool_context_is_derived_from_one_authority_snapshot() {
        let authority = ExecutionAuthority::new(
            PathBuf::from("workspace"),
            PermissionMode::ReadOnly,
            LocalSandboxConfig::default(),
            CapabilityProjection::unrestricted(),
        )
        .unwrap();
        let context = authority.local_tool_context();
        assert_eq!(context.permission_mode, PermissionMode::ReadOnly);
        assert_eq!(
            context.sandbox_config.as_ref(),
            Some(authority.sandbox_config())
        );
        assert_eq!(
            context.capability_projection,
            *authority.capability_projection()
        );
    }

    #[test]
    fn sandbox_authority_can_only_be_narrowed() {
        let authority = ExecutionAuthority::new(
            PathBuf::from("workspace"),
            PermissionMode::ReadOnly,
            LocalSandboxConfig::default().with_sandbox_mode(crate::sandbox::SandboxMode::ReadOnly),
            CapabilityProjection::unrestricted(),
        )
        .unwrap();
        assert!(authority
            .with_sandbox(
                LocalSandboxConfig::default()
                    .with_sandbox_mode(crate::sandbox::SandboxMode::DangerFullAccess),
            )
            .is_err());
    }
}
