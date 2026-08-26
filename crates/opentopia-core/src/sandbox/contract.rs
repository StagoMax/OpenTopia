//! Stable sandbox policy and command-plan contracts.

use super::path_policy::{
    absolute_path, canonicalize_existing_ancestor, dedup_paths, env_path_list,
    parse_enforcement_mode, path_is_within_approved_scope, paths_equal,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowsSandboxBackend {
    #[default]
    Auto,
    #[serde(alias = "elevated")]
    DedicatedUser,
    Unelevated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowsSandboxSetupState {
    Unavailable,
    NotConfigured,
    Ready,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSandboxSetupComponents {
    pub credentials: bool,
    pub offline_identity: bool,
    pub online_identity: bool,
    pub offline_network_policy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxBackendCapabilities {
    pub recursive_read_allowlist: bool,
    pub recursive_write_allowlist: bool,
    pub deny_read: bool,
    pub deny_write: bool,
    pub network_offline: bool,
    pub network_online: bool,
    pub private_desktop: bool,
    /// The backend preserves ordinary child-process IPC such as anonymous and
    /// named pipes used by language runtimes to capture nested process output.
    pub native_subprocess_ipc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSandboxSetupStatus {
    pub supported: bool,
    pub helper_available: bool,
    pub state: WindowsSandboxSetupState,
    pub backend: String,
    pub state_dir: Option<String>,
    pub components: WindowsSandboxSetupComponents,
    pub issues: Vec<String>,
}

impl WindowsSandboxSetupStatus {
    pub fn is_ready(&self) -> bool {
        self.state == WindowsSandboxSetupState::Ready
    }
}

impl SandboxBackendCapabilities {
    pub fn for_platform(platform: OsSandboxPlatform, backend: WindowsSandboxBackend) -> Self {
        match platform {
            OsSandboxPlatform::Windows => match backend {
                WindowsSandboxBackend::DedicatedUser => Self {
                    // A dedicated local user removes access to the host user's
                    // private profile and supports explicit deny-read ACEs, but
                    // Windows still has machine paths readable by all Users.
                    // Do not advertise a complete read allowlist.
                    recursive_read_allowlist: false,
                    // Windows runtime initialization needs broad compatibility
                    // SIDs in the restricted-token check. The dedicated user
                    // still isolates host-private locations and supports
                    // explicit protected-root denies, but is not a complete
                    // host-wide write allowlist.
                    recursive_write_allowlist: false,
                    deny_read: true,
                    deny_write: true,
                    network_offline: true,
                    network_online: true,
                    // Do not claim a private interactive desktop until the
                    // backend creates and assigns one.
                    private_desktop: false,
                    native_subprocess_ipc: true,
                },
                WindowsSandboxBackend::Auto | WindowsSandboxBackend::Unelevated => Self {
                    // WRITE_RESTRICTED constrains writes, not reads. The
                    // fallback deliberately preserves normal host-user reads.
                    recursive_read_allowlist: false,
                    recursive_write_allowlist: true,
                    deny_read: false,
                    deny_write: true,
                    network_offline: false,
                    network_online: true,
                    private_desktop: false,
                    native_subprocess_ipc: false,
                },
            },
            OsSandboxPlatform::Linux | OsSandboxPlatform::Macos => Self {
                recursive_read_allowlist: true,
                recursive_write_allowlist: true,
                // Existing bwrap/Seatbelt profiles expose an allowlist, but
                // the portable request layer does not yet translate arbitrary
                // per-command deny-read exceptions on these platforms.
                deny_read: false,
                deny_write: true,
                network_offline: true,
                network_online: true,
                private_desktop: false,
                native_subprocess_ipc: true,
            },
            OsSandboxPlatform::Unsupported => Self {
                recursive_read_allowlist: false,
                recursive_write_allowlist: false,
                deny_read: false,
                deny_write: false,
                network_offline: false,
                network_online: false,
                private_desktop: false,
                native_subprocess_ipc: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironmentKind {
    Local,
    Docker,
    Remote,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxLifecycle {
    Ready,
    Starting,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OsSandboxMode {
    Disabled,
    BestEffort,
    Enforce,
}

impl Default for OsSandboxMode {
    fn default() -> Self {
        Self::Disabled
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Inherit,
    Allow,
    Deny,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self::Deny
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    pub fn is_attenuation_of(self, parent: Self) -> bool {
        let rank = |mode| match mode {
            Self::ReadOnly => 0,
            Self::WorkspaceWrite => 1,
            Self::DangerFullAccess => 2,
        };
        rank(self) <= rank(parent)
    }
}

impl Default for SandboxMode {
    fn default() -> Self {
        Self::WorkspaceWrite
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalSandboxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: OsSandboxMode,
    #[serde(default)]
    pub network: NetworkPolicy,
    #[serde(default)]
    pub read_paths: Vec<PathBuf>,
    #[serde(default)]
    pub write_paths: Vec<PathBuf>,
    #[serde(default)]
    pub sandbox_mode: SandboxMode,
    #[serde(default)]
    pub writable_roots: Vec<PathBuf>,
    #[serde(default)]
    pub sandbox_home: Option<PathBuf>,
    #[serde(default)]
    pub windows_backend: WindowsSandboxBackend,
    /// Exact read paths projected into the active authorization scope. Reads
    /// do not require approval, but Windows needs these call-scoped paths to
    /// provision access for its dedicated low-privilege user.
    #[serde(skip)]
    pub approved_read_paths: Vec<PathBuf>,
    /// Exact write paths approved by the active authorization scope. The holder
    /// decides the lifetime: a one-call replay or a turn-scoped path lease.
    #[serde(skip)]
    pub approved_write_paths: Vec<PathBuf>,
}

impl Default for LocalSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OsSandboxMode::Disabled,
            network: NetworkPolicy::Deny,
            read_paths: Vec::new(),
            write_paths: Vec::new(),
            sandbox_mode: SandboxMode::WorkspaceWrite,
            writable_roots: Vec::new(),
            sandbox_home: None,
            windows_backend: WindowsSandboxBackend::Auto,
            approved_read_paths: Vec::new(),
            approved_write_paths: Vec::new(),
        }
    }
}

impl LocalSandboxConfig {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn best_effort() -> Self {
        Self {
            enabled: true,
            mode: OsSandboxMode::BestEffort,
            sandbox_mode: SandboxMode::WorkspaceWrite,
            ..Self::default()
        }
    }

    pub fn enforce() -> Self {
        Self {
            enabled: true,
            mode: OsSandboxMode::Enforce,
            sandbox_mode: SandboxMode::WorkspaceWrite,
            ..Self::default()
        }
    }

    pub fn danger_full_access() -> Self {
        Self {
            enabled: false,
            mode: OsSandboxMode::Disabled,
            network: NetworkPolicy::Allow,
            sandbox_mode: SandboxMode::DangerFullAccess,
            ..Self::default()
        }
    }

    /// Resolve the policy-level `auto` choice before capability checks and
    /// command construction. Enforce mode requires the complete dedicated-user
    /// contract; best-effort mode may use the compatibility-limited restricted
    /// token backend.
    pub fn effective_windows_backend(&self) -> WindowsSandboxBackend {
        match (self.windows_backend, self.mode) {
            (WindowsSandboxBackend::Auto, OsSandboxMode::Enforce) => {
                WindowsSandboxBackend::DedicatedUser
            }
            (WindowsSandboxBackend::Auto, OsSandboxMode::BestEffort) => WindowsSandboxBackend::Auto,
            (backend, _) => backend,
        }
    }

    pub fn with_sandbox_mode(mut self, sandbox_mode: SandboxMode) -> Self {
        self.sandbox_mode = sandbox_mode;
        if sandbox_mode == SandboxMode::DangerFullAccess {
            self.enabled = false;
            self.mode = OsSandboxMode::Disabled;
            self.network = NetworkPolicy::Allow;
        }
        self
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.mode != OsSandboxMode::Disabled
    }

    pub fn is_enforced(&self) -> bool {
        self.enabled && self.mode == OsSandboxMode::Enforce
    }

    pub fn grant_read_path(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if !self
            .approved_read_paths
            .iter()
            .any(|approved| paths_equal(&path, approved))
        {
            self.approved_read_paths.push(path);
        }
    }

    pub fn grant_write_path(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        if !self
            .approved_write_paths
            .iter()
            .any(|approved| paths_equal(&path, approved))
        {
            self.approved_write_paths.push(path);
        }
    }

    pub fn is_within_approved_read_scope(&self, path: &Path) -> bool {
        self.approved_read_paths
            .iter()
            .any(|approved| path_is_within_approved_scope(path, approved))
    }

    pub fn is_within_approved_write_scope(&self, path: &Path) -> bool {
        self.approved_write_paths
            .iter()
            .any(|approved| path_is_within_approved_scope(path, approved))
    }

    pub fn is_approved_write_path(&self, path: &Path) -> bool {
        self.approved_write_paths
            .iter()
            .any(|approved| paths_equal(path, approved))
    }

    pub fn has_approved_write_within(&self, root: &Path) -> bool {
        let root = canonicalize_existing_ancestor(&absolute_path(root));
        self.approved_write_paths.iter().any(|approved| {
            canonicalize_existing_ancestor(&absolute_path(approved)).starts_with(&root)
        })
    }

    pub fn from_env() -> Self {
        let mode_value = std::env::var("OPENTOPIA_SANDBOX_MODE")
            .unwrap_or_else(|_| "workspace-write".to_string())
            .to_ascii_lowercase()
            .replace('_', "-");
        let (legacy_enforcement, sandbox_mode) = match mode_value.as_str() {
            "enforce" | "strict" => (Some(OsSandboxMode::Enforce), SandboxMode::WorkspaceWrite),
            "best-effort" => (Some(OsSandboxMode::BestEffort), SandboxMode::WorkspaceWrite),
            "disabled" => (Some(OsSandboxMode::Disabled), SandboxMode::DangerFullAccess),
            "read-only" => (None, SandboxMode::ReadOnly),
            "workspace-write" => (None, SandboxMode::WorkspaceWrite),
            "danger-full-access" => (None, SandboxMode::DangerFullAccess),
            _ => (Some(OsSandboxMode::Enforce), SandboxMode::ReadOnly),
        };
        let mode = std::env::var("OPENTOPIA_SANDBOX_ENFORCEMENT")
            .ok()
            .and_then(|value| parse_enforcement_mode(&value))
            .or(legacy_enforcement)
            .unwrap_or_else(|| {
                if sandbox_mode == SandboxMode::DangerFullAccess {
                    OsSandboxMode::Disabled
                } else {
                    OsSandboxMode::Enforce
                }
            });
        let configured_network = match std::env::var("OPENTOPIA_SANDBOX_NETWORK")
            .unwrap_or_else(|_| "deny".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "allow" => NetworkPolicy::Allow,
            "inherit" => NetworkPolicy::Inherit,
            _ => NetworkPolicy::Deny,
        };
        let network = if sandbox_mode == SandboxMode::DangerFullAccess {
            NetworkPolicy::Allow
        } else {
            configured_network
        };
        Self {
            enabled: mode != OsSandboxMode::Disabled
                && sandbox_mode != SandboxMode::DangerFullAccess,
            mode,
            network,
            read_paths: env_path_list("OPENTOPIA_SANDBOX_READ_PATHS"),
            write_paths: env_path_list("OPENTOPIA_SANDBOX_WRITE_PATHS"),
            sandbox_mode,
            writable_roots: env_path_list("OPENTOPIA_SANDBOX_WRITABLE_ROOTS"),
            sandbox_home: std::env::var("OPENTOPIA_SANDBOX_HOME")
                .ok()
                .map(PathBuf::from),
            windows_backend: match std::env::var("OPENTOPIA_WINDOWS_SANDBOX") {
                Ok(value) => match value.to_ascii_lowercase().as_str() {
                    "auto" => WindowsSandboxBackend::Auto,
                    "dedicated_user" | "dedicated-user" | "elevated" => {
                        WindowsSandboxBackend::DedicatedUser
                    }
                    "unelevated" | "legacy" => WindowsSandboxBackend::Unelevated,
                    // Fail closed: an invalid backend name must not silently
                    // select the weaker fallback.
                    _ => WindowsSandboxBackend::DedicatedUser,
                },
                Err(_) => WindowsSandboxBackend::Auto,
            },
            approved_read_paths: Vec::new(),
            approved_write_paths: Vec::new(),
        }
    }

    pub fn effective_writable_roots(&self, workspace_root: &Path) -> Vec<PathBuf> {
        if self.sandbox_mode != SandboxMode::WorkspaceWrite {
            return Vec::new();
        }
        dedup_paths(
            std::iter::once(workspace_root.to_path_buf())
                .chain(self.write_paths.iter().cloned())
                .chain(self.writable_roots.iter().cloned())
                .chain(
                    self.approved_write_paths
                        .iter()
                        .filter_map(|path| path.parent().map(Path::to_path_buf)),
                ),
        )
    }

    /// Policy/runtime roots configured independently of an approval. Exact
    /// approval paths are checked separately so authorizing one file cannot
    /// accidentally authorize its siblings through the parent mount.
    pub(crate) fn configured_writable_roots(&self, workspace_root: &Path) -> Vec<PathBuf> {
        if self.sandbox_mode != SandboxMode::WorkspaceWrite {
            return Vec::new();
        }
        dedup_paths(
            std::iter::once(workspace_root.to_path_buf())
                .chain(self.write_paths.iter().cloned())
                .chain(self.writable_roots.iter().cloned()),
        )
    }

    pub fn effective_sandbox_home(&self, workspace_root: &Path) -> Option<PathBuf> {
        if !self.is_enabled() {
            return self.sandbox_home.clone();
        }
        self.sandbox_home.clone().or_else(|| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            absolute_path(workspace_root).hash(&mut hasher);
            Some(
                std::env::temp_dir()
                    .join("OpenTopia")
                    .join("sandbox-home")
                    .join(format!("{:016x}", hasher.finish())),
            )
        })
    }

    pub fn effective_command_writable_roots(&self, workspace_root: &Path) -> Vec<PathBuf> {
        dedup_paths(
            self.effective_writable_roots(workspace_root)
                .into_iter()
                .chain(self.effective_sandbox_home(workspace_root)),
        )
    }

    pub fn effective_command_readable_roots(&self, workspace_root: &Path) -> Vec<PathBuf> {
        dedup_paths(
            self.effective_readable_roots(workspace_root)
                .into_iter()
                .chain(self.effective_sandbox_home(workspace_root)),
        )
    }

    pub fn effective_readable_roots(&self, workspace_root: &Path) -> Vec<PathBuf> {
        if self.sandbox_mode == SandboxMode::DangerFullAccess {
            return Vec::new();
        }
        dedup_paths(
            std::iter::once(workspace_root.to_path_buf())
                .chain(self.read_paths.iter().cloned())
                .chain(self.approved_read_paths.iter().cloned())
                .chain(self.approved_write_paths.iter().cloned())
                .chain(self.effective_writable_roots(workspace_root)),
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OsSandboxPlatform {
    Linux,
    Macos,
    Windows,
    Unsupported,
}

impl OsSandboxPlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Unsupported
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SandboxCommandStatus {
    Disabled,
    Unrestricted,
    Wrapped {
        platform: OsSandboxPlatform,
        backend: String,
    },
    BestEffortPassthrough {
        platform: OsSandboxPlatform,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCommandPlan {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Stable account/runtime setup that is shared by many filesystem scopes.
    /// It is prepared before the scope-specific phase below.
    #[serde(default)]
    pub baseline_preparation: Option<SandboxPreparationPlan>,
    #[serde(default)]
    pub preparation: Option<SandboxPreparationPlan>,
    pub status: SandboxCommandStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPreparationPlan {
    pub key: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct SandboxLaunchOptions {
    pub interactive: bool,
    /// The child must retain bidirectional stdio for its full lifetime. On
    /// Windows this selects the streaming restricted-token backend because the
    /// dedicated-user broker currently supports one-shot file capture only.
    pub persistent_stdio: bool,
    pub runtime_read_roots: Vec<PathBuf>,
    pub managed_runtime_roots: Vec<PathBuf>,
    pub environment_keys: Vec<String>,
    pub additional_denied_read_paths: Vec<PathBuf>,
    pub additional_protected_paths: Vec<PathBuf>,
    pub timeout_ms: Option<u64>,
    pub termination_timeout_ms: Option<u64>,
    pub max_memory_bytes: Option<u64>,
    pub max_cpu_time_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
}

impl SandboxCommandPlan {
    pub(super) fn disabled(program: &str, args: &[String]) -> Self {
        Self {
            program: program.to_string(),
            args: args.to_vec(),
            env: Vec::new(),
            baseline_preparation: None,
            preparation: None,
            status: SandboxCommandStatus::Disabled,
        }
    }

    pub(super) fn unrestricted(program: &str, args: &[String]) -> Self {
        Self {
            program: program.to_string(),
            args: args.to_vec(),
            env: Vec::new(),
            baseline_preparation: None,
            preparation: None,
            status: SandboxCommandStatus::Unrestricted,
        }
    }
}
