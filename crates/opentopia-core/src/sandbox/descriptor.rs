//! Runtime descriptor projection for the configured local sandbox.

use super::command::{
    build_local_sandbox_command_for_platform, sandbox_permission_profile, sandbox_probe_command,
};
use super::contract::{
    ExecutionEnvironmentKind, LocalSandboxConfig, NetworkPolicy, OsSandboxMode, OsSandboxPlatform,
    SandboxCommandPlan, SandboxCommandStatus, SandboxLifecycle, SandboxMode,
};
use super::path_policy::protected_paths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SandboxDescriptor {
    pub id: String,
    pub thread_id: Uuid,
    pub kind: ExecutionEnvironmentKind,
    pub lifecycle: SandboxLifecycle,
    pub workspace_root: PathBuf,
    pub capabilities: Vec<String>,
    pub message: String,
    pub platform: OsSandboxPlatform,
    pub mode: OsSandboxMode,
    pub network: NetworkPolicy,
    pub sandbox_mode: SandboxMode,
    pub readable_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    pub protected_paths: Vec<PathBuf>,
    pub backend: Option<String>,
    pub permission_profile: String,
    pub enforced: bool,
    pub available: bool,
}

impl SandboxDescriptor {
    pub fn local(thread_id: Uuid, workspace_root: PathBuf, config: &LocalSandboxConfig) -> Self {
        let platform = OsSandboxPlatform::current();
        let probe = sandbox_probe_command(platform);
        let plan = build_local_sandbox_command_for_platform(
            platform,
            &probe.0,
            &probe.1,
            &workspace_root,
            &workspace_root,
            config,
        );
        let (lifecycle, available, enforced, backend, message) = match plan {
            Ok(SandboxCommandPlan {
                status: SandboxCommandStatus::Wrapped { backend, .. },
                ..
            }) => (
                SandboxLifecycle::Ready,
                true,
                true,
                Some(backend.clone()),
                format!(
                    "OS sandbox command wrapping is configured using {backend}; restricted calls fail closed at execution time."
                ),
            ),
            Ok(SandboxCommandPlan {
                status: SandboxCommandStatus::BestEffortPassthrough { reason, .. },
                ..
            }) => (SandboxLifecycle::Ready, false, false, None, reason),
            Ok(SandboxCommandPlan {
                status: SandboxCommandStatus::Unrestricted,
                ..
            }) => (
                SandboxLifecycle::Ready,
                true,
                false,
                None,
                "Sandbox restrictions are disabled; commands have full filesystem and network access."
                    .to_string(),
            ),
            Ok(_) => (
                SandboxLifecycle::Stopped,
                false,
                false,
                None,
                "OS sandbox is disabled by configuration.".to_string(),
            ),
            Err(err) => (SandboxLifecycle::Error, false, false, None, err.to_string()),
        };
        let readable_roots = config.effective_readable_roots(&workspace_root);
        let writable_roots = config.effective_writable_roots(&workspace_root);
        let protected_paths = protected_paths(&workspace_root, config);
        Self {
            id: format!("local-{thread_id}"),
            thread_id,
            kind: ExecutionEnvironmentKind::Local,
            lifecycle,
            workspace_root,
            capabilities: sandbox_capabilities(config.sandbox_mode),
            message,
            platform,
            mode: config.mode,
            network: config.network,
            sandbox_mode: config.sandbox_mode,
            readable_roots,
            writable_roots,
            protected_paths,
            backend,
            permission_profile: sandbox_permission_profile(platform, config),
            enforced,
            available,
        }
    }
}

fn sandbox_capabilities(mode: SandboxMode) -> Vec<String> {
    let mut capabilities = vec![
        "filesystem".to_string(),
        "shell".to_string(),
        "spawn_stdio".to_string(),
        "os_sandbox_preflight".to_string(),
    ];
    if mode != SandboxMode::ReadOnly {
        capabilities.push("apply_patch".to_string());
    }
    capabilities
}
