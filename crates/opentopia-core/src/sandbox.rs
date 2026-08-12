use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowsSandboxBackend {
    #[default]
    Auto,
    #[serde(alias = "elevated")]
    DedicatedUser,
    Unelevated,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowsSandboxSetupState {
    Unavailable,
    NotConfigured,
    Ready,
    Degraded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
                    recursive_write_allowlist: true,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironmentKind {
    Local,
    Docker,
    Remote,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxLifecycle {
    Ready,
    Starting,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Exact paths approved only for the replay of one user-approved tool call.
    #[serde(skip)]
    pub approved_read_paths: Vec<PathBuf>,
    /// Exact paths approved only for the replay of one user-approved tool call.
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
        self.approved_read_paths.push(path.into());
    }

    pub fn grant_write_path(&mut self, path: impl Into<PathBuf>) {
        self.approved_write_paths.push(path.into());
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
                .chain(self.effective_writable_roots(workspace_root)),
        )
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
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

fn canonicalize_existing_ancestor(path: &Path) -> PathBuf {
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

#[cfg(windows)]
fn windows_comparison_path(path: &Path) -> String {
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

fn parse_enforcement_mode(value: &str) -> Option<OsSandboxMode> {
    match value.to_ascii_lowercase().replace('_', "-").as_str() {
        "disabled" => Some(OsSandboxMode::Disabled),
        "best-effort" => Some(OsSandboxMode::BestEffort),
        "enforce" | "strict" => Some(OsSandboxMode::Enforce),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    pub status: SandboxCommandStatus,
}

#[derive(Debug, Clone, Default)]
pub struct SandboxLaunchOptions {
    pub interactive: bool,
    pub runtime_read_roots: Vec<PathBuf>,
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
    fn disabled(program: &str, args: &[String]) -> Self {
        Self {
            program: program.to_string(),
            args: args.to_vec(),
            env: Vec::new(),
            status: SandboxCommandStatus::Disabled,
        }
    }

    fn unrestricted(program: &str, args: &[String]) -> Self {
        Self {
            program: program.to_string(),
            args: args.to_vec(),
            env: Vec::new(),
            status: SandboxCommandStatus::Unrestricted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub fn sandbox_permission_profile(
    platform: OsSandboxPlatform,
    config: &LocalSandboxConfig,
) -> String {
    match platform {
        OsSandboxPlatform::Windows => windows_permission_profile(config).to_string(),
        _ => config.sandbox_mode.as_str().to_string(),
    }
}

fn sandbox_probe_command(platform: OsSandboxPlatform) -> (String, Vec<String>) {
    match platform {
        OsSandboxPlatform::Windows => (
            "cmd.exe".to_string(),
            vec!["/d".to_string(), "/c".to_string(), "exit 0".to_string()],
        ),
        _ => ("/usr/bin/true".to_string(), Vec::new()),
    }
}

pub fn build_local_sandbox_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
) -> anyhow::Result<SandboxCommandPlan> {
    build_local_sandbox_command_with_options(
        program,
        args,
        cwd,
        workspace_root,
        config,
        &SandboxLaunchOptions::default(),
    )
}

pub fn build_local_sandbox_command_with_options(
    program: &str,
    args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
    options: &SandboxLaunchOptions,
) -> anyhow::Result<SandboxCommandPlan> {
    build_local_sandbox_command_for_platform_with_options(
        OsSandboxPlatform::current(),
        program,
        args,
        cwd,
        workspace_root,
        config,
        options,
    )
}

pub fn build_local_sandbox_command_for_platform(
    platform: OsSandboxPlatform,
    program: &str,
    args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
) -> anyhow::Result<SandboxCommandPlan> {
    build_local_sandbox_command_for_platform_with_options(
        platform,
        program,
        args,
        cwd,
        workspace_root,
        config,
        &SandboxLaunchOptions::default(),
    )
}

pub fn build_local_sandbox_command_for_platform_with_options(
    platform: OsSandboxPlatform,
    program: &str,
    args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
    options: &SandboxLaunchOptions,
) -> anyhow::Result<SandboxCommandPlan> {
    if config.sandbox_mode == SandboxMode::DangerFullAccess {
        return Ok(SandboxCommandPlan::unrestricted(program, args));
    }
    if !config.is_enabled() {
        return Ok(SandboxCommandPlan::disabled(program, args));
    }

    match platform {
        OsSandboxPlatform::Linux => {
            build_bubblewrap_command(program, args, cwd, workspace_root, config)
        }
        OsSandboxPlatform::Macos => {
            build_sandbox_exec_command(program, args, workspace_root, config)
        }
        OsSandboxPlatform::Windows => {
            build_windows_sandbox_command(program, args, cwd, workspace_root, config, options)
        }
        OsSandboxPlatform::Unsupported => {
            build_unsupported_sandbox_command(platform, program, args, config)
        }
    }
}

fn build_bubblewrap_command(
    program: &str,
    original_args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
) -> anyhow::Result<SandboxCommandPlan> {
    let backend = if OsSandboxPlatform::current() == OsSandboxPlatform::Linux {
        first_existing_executable(&[PathBuf::from("/usr/bin/bwrap"), PathBuf::from("/bin/bwrap")])
    } else {
        Some(PathBuf::from("/usr/bin/bwrap"))
    };
    let Some(backend) = backend else {
        return unavailable_backend(
            OsSandboxPlatform::Linux,
            "bubblewrap was not found at /usr/bin/bwrap or /bin/bwrap",
            program,
            original_args,
            config,
        );
    };
    let workspace_root = absolute_path(workspace_root);
    let cwd = absolute_path(cwd);
    let mut args = vec![
        "--die-with-parent".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-ipc".to_string(),
        "--unshare-uts".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--tmpfs".to_string(),
        "/tmp".to_string(),
    ];

    if config.network == NetworkPolicy::Deny {
        args.push("--unshare-net".to_string());
    }

    for path in default_system_read_paths() {
        args.push("--ro-bind".to_string());
        args.push(path.to_string());
        args.push(path.to_string());
    }

    for path in config.effective_readable_roots(&workspace_root) {
        let path = absolute_path(&path);
        args.push("--ro-bind".to_string());
        args.push(path_to_string(&path));
        args.push(path_to_string(&path));
    }

    for path in config.effective_writable_roots(&workspace_root) {
        let path = absolute_path(&path);
        args.push("--bind".to_string());
        args.push(path_to_string(&path));
        args.push(path_to_string(&path));
    }

    for path in protected_paths(&workspace_root, config)
        .into_iter()
        .filter(|path| path.exists())
    {
        let path = absolute_path(path);
        args.push("--ro-bind".to_string());
        args.push(path_to_string(&path));
        args.push(path_to_string(&path));
    }

    args.push("--chdir".to_string());
    args.push(path_to_string(&cwd));
    args.push("--".to_string());
    args.push(program.to_string());
    args.extend(original_args.iter().cloned());

    Ok(SandboxCommandPlan {
        program: path_to_string(&backend),
        args,
        env: Vec::new(),
        status: SandboxCommandStatus::Wrapped {
            platform: OsSandboxPlatform::Linux,
            backend: "bubblewrap".to_string(),
        },
    })
}

fn build_sandbox_exec_command(
    program: &str,
    original_args: &[String],
    workspace_root: &Path,
    config: &LocalSandboxConfig,
) -> anyhow::Result<SandboxCommandPlan> {
    let backend = PathBuf::from("/usr/bin/sandbox-exec");
    if OsSandboxPlatform::current() == OsSandboxPlatform::Macos && !backend.is_file() {
        return unavailable_backend(
            OsSandboxPlatform::Macos,
            "/usr/bin/sandbox-exec is unavailable",
            program,
            original_args,
            config,
        );
    }
    let profile = seatbelt_profile(workspace_root, config);
    let mut args = vec!["-p".to_string(), profile, program.to_string()];
    args.extend(original_args.iter().cloned());

    Ok(SandboxCommandPlan {
        program: path_to_string(&backend),
        args,
        env: Vec::new(),
        status: SandboxCommandStatus::Wrapped {
            platform: OsSandboxPlatform::Macos,
            backend: "seatbelt".to_string(),
        },
    })
}

fn build_windows_sandbox_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
    options: &SandboxLaunchOptions,
) -> anyhow::Result<SandboxCommandPlan> {
    let sandbox = match resolve_opentopia_sandbox_binary() {
        Ok(Some(sandbox)) => sandbox,
        Ok(None) => {
            let reason = std::env::var("OPENTOPIA_SANDBOX_BACKEND_ERROR")
                .unwrap_or_else(|_| "OpenTopia Windows sandbox backend was not found".to_string());
            return unavailable_backend(OsSandboxPlatform::Windows, reason, program, args, config);
        }
        Err(error) => {
            return unavailable_backend(
                OsSandboxPlatform::Windows,
                error.to_string(),
                program,
                args,
                config,
            )
        }
    };
    build_windows_sandbox_command_with_binary(
        sandbox,
        program,
        args,
        cwd,
        workspace_root,
        config,
        options,
    )
}

fn build_windows_sandbox_command_with_binary(
    sandbox: PathBuf,
    program: &str,
    args: &[String],
    cwd: &Path,
    workspace_root: &Path,
    config: &LocalSandboxConfig,
    options: &SandboxLaunchOptions,
) -> anyhow::Result<SandboxCommandPlan> {
    const ERROR_NONCE_ENV: &str = "OPENTOPIA_SANDBOX_ERROR_NONCE";
    let workspace_root = absolute_path(workspace_root);
    let error_nonce = Uuid::new_v4().simple().to_string();
    let backend = config.effective_windows_backend();
    if config.mode == OsSandboxMode::Enforce && backend != WindowsSandboxBackend::DedicatedUser {
        anyhow::bail!(
            "Windows enforce mode requires the dedicated-user sandbox backend because the restricted-token backend cannot preserve arbitrary child-process IPC; choose auto/dedicated_user and complete `opentopia-sandbox setup`, or explicitly use best-effort mode"
        );
    }
    let mut sandbox_args = vec![
        "run".to_string(),
        "--cwd".to_string(),
        path_to_string(&absolute_path(cwd)),
        "--backend".to_string(),
        match backend {
            WindowsSandboxBackend::Auto => "auto",
            WindowsSandboxBackend::DedicatedUser => "dedicated-user",
            WindowsSandboxBackend::Unelevated => "unelevated",
        }
        .to_string(),
    ];
    if options.interactive {
        sandbox_args.push("--interactive".to_string());
    }
    if let Some(timeout_ms) = options.timeout_ms {
        sandbox_args.extend(["--timeout-ms".to_string(), timeout_ms.to_string()]);
    }
    if let Some(timeout_ms) = options.termination_timeout_ms {
        sandbox_args.extend([
            "--termination-timeout-ms".to_string(),
            timeout_ms.to_string(),
        ]);
    }
    if let Some(bytes) = options.max_memory_bytes {
        sandbox_args.extend(["--max-memory-bytes".to_string(), bytes.to_string()]);
    }
    if let Some(milliseconds) = options.max_cpu_time_ms {
        sandbox_args.extend(["--max-cpu-time-ms".to_string(), milliseconds.to_string()]);
    }
    if let Some(bytes) = options.max_output_bytes {
        sandbox_args.extend(["--max-output-bytes".to_string(), bytes.to_string()]);
    }
    for root in config
        .effective_command_readable_roots(&workspace_root)
        .into_iter()
        .filter(|root| root.exists())
    {
        sandbox_args.extend([
            "--read-root".to_string(),
            path_to_string(&absolute_path(root)),
        ]);
    }
    for root in options
        .runtime_read_roots
        .iter()
        .cloned()
        .chain(windows_minimal_runtime_roots())
        .filter(|root| root.exists())
    {
        sandbox_args.extend([
            "--runtime-root".to_string(),
            path_to_string(&absolute_path(root)),
        ]);
    }
    for root in config
        .effective_command_writable_roots(&workspace_root)
        .into_iter()
        .filter(|root| root.exists())
    {
        sandbox_args.extend([
            "--write-root".to_string(),
            path_to_string(&absolute_path(root)),
        ]);
    }
    if let Some(home) = config
        .effective_sandbox_home(&workspace_root)
        .filter(|path| path.exists())
    {
        sandbox_args.extend([
            "--runtime-home".to_string(),
            path_to_string(&absolute_path(home)),
        ]);
    }
    let (protected, approved_protected): (Vec<_>, Vec<_>) =
        protected_paths(&workspace_root, config)
            .into_iter()
            .partition(|path| !config.has_approved_write_within(path));
    for path in approved_protected.into_iter().filter(|path| path.exists()) {
        sandbox_args.extend([
            "--allow-protected-root".to_string(),
            path_to_string(&absolute_path(path)),
        ]);
    }
    for path in protected
        .into_iter()
        .chain(options.additional_protected_paths.iter().cloned())
        .filter(|path| path.exists())
    {
        sandbox_args.extend([
            "--protect".to_string(),
            path_to_string(&absolute_path(path)),
        ]);
    }
    for path in options
        .additional_denied_read_paths
        .iter()
        .filter(|path| path.exists())
    {
        sandbox_args.extend([
            "--deny-read".to_string(),
            path_to_string(&absolute_path(path)),
        ]);
    }
    sandbox_args.extend([
        "--network".to_string(),
        match config.network {
            NetworkPolicy::Deny => "deny",
            NetworkPolicy::Allow | NetworkPolicy::Inherit => "internet",
        }
        .to_string(),
    ]);
    sandbox_args.push("--".to_string());
    sandbox_args.push(program.to_string());
    sandbox_args.extend(args.iter().cloned());

    Ok(SandboxCommandPlan {
        program: path_to_string(&sandbox),
        args: sandbox_args,
        env: {
            let mut env = opentopia_sandbox_state_dir()
                .map(|path| {
                    vec![(
                        "OPENTOPIA_SANDBOX_STATE_DIR".to_string(),
                        path_to_string(&path),
                    )]
                })
                .unwrap_or_default();
            let mut keys = windows_sandbox_environment_keys();
            keys.extend(options.environment_keys.iter().cloned());
            keys.push("OPENTOPIA_SANDBOX_STATE_DIR".to_string());
            keys.sort_by_key(|key| key.to_ascii_uppercase());
            keys.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
            env.push(("OPENTOPIA_SANDBOX_ENV_KEYS".to_string(), keys.join(";")));
            env.push((ERROR_NONCE_ENV.to_string(), error_nonce));
            env
        },
        status: SandboxCommandStatus::Wrapped {
            platform: OsSandboxPlatform::Windows,
            backend: match backend {
                WindowsSandboxBackend::Auto => "opentopia-windows-auto",
                WindowsSandboxBackend::DedicatedUser => "opentopia-windows-dedicated-user",
                WindowsSandboxBackend::Unelevated => "opentopia-windows-restricted-token",
            }
            .to_string(),
        },
    })
}

fn windows_sandbox_environment_keys() -> Vec<String> {
    [
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATH",
        "PATHEXT",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "USERPROFILE",
        "HOME",
        "XDG_CONFIG_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "HOMEDRIVE",
        "HOMEPATH",
        "NO_COLOR",
        "TERM",
        "PAGER",
        "GIT_PAGER",
        "GH_PAGER",
        "CI",
        "OPENTOPIA_SANDBOX",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn opentopia_sandbox_state_dir() -> Option<PathBuf> {
    std::env::var_os("OPENTOPIA_SANDBOX_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|root| root.join("OpenTopia").join("sandbox"))
        })
}

#[cfg(all(test, windows))]
pub(crate) fn dedicated_user_credentials_are_installed_for_tests() -> bool {
    opentopia_sandbox_state_dir().is_some_and(|path| path.join("credentials.dpapi").is_file())
}

fn windows_minimal_runtime_roots() -> impl Iterator<Item = PathBuf> {
    [
        std::env::var_os("SystemRoot").map(PathBuf::from),
        std::env::var_os("ProgramFiles").map(PathBuf::from),
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        std::env::var_os("ProgramData").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .filter(|path| path.exists())
}

fn build_unsupported_sandbox_command(
    platform: OsSandboxPlatform,
    program: &str,
    args: &[String],
    config: &LocalSandboxConfig,
) -> anyhow::Result<SandboxCommandPlan> {
    let reason = format!(
        "OS-level local sandboxing is unsupported on platform '{}'.",
        platform.as_str()
    );
    match config.mode {
        OsSandboxMode::Disabled => Ok(SandboxCommandPlan::disabled(program, args)),
        OsSandboxMode::BestEffort => Ok(SandboxCommandPlan {
            program: program.to_string(),
            args: args.to_vec(),
            env: Vec::new(),
            status: SandboxCommandStatus::BestEffortPassthrough { platform, reason },
        }),
        OsSandboxMode::Enforce => anyhow::bail!("{reason}"),
    }
}

fn unavailable_backend(
    platform: OsSandboxPlatform,
    reason: impl Into<String>,
    program: &str,
    args: &[String],
    config: &LocalSandboxConfig,
) -> anyhow::Result<SandboxCommandPlan> {
    let reason = reason.into();
    if config.is_enforced() {
        anyhow::bail!("{reason}");
    }
    Ok(SandboxCommandPlan {
        program: program.to_string(),
        args: args.to_vec(),
        env: Vec::new(),
        status: SandboxCommandStatus::BestEffortPassthrough { platform, reason },
    })
}

fn windows_permission_profile(config: &LocalSandboxConfig) -> &'static str {
    match (config.sandbox_mode, config.network) {
        (SandboxMode::ReadOnly, NetworkPolicy::Deny) => "opentopia-windows-read-only-offline",
        (SandboxMode::WorkspaceWrite, NetworkPolicy::Deny) => {
            "opentopia-windows-workspace-write-offline"
        }
        (SandboxMode::ReadOnly, _) => "opentopia-windows-read-only-internet",
        (SandboxMode::WorkspaceWrite, _) => "opentopia-windows-workspace-write-internet",
        (SandboxMode::DangerFullAccess, _) => "danger-full-access",
    }
}

static WINDOWS_SANDBOX_PROTOCOL_CACHE: OnceLock<Mutex<HashMap<String, Result<(), String>>>> =
    OnceLock::new();

fn sandbox_binary_fingerprint(path: &Path) -> anyhow::Result<String> {
    let metadata = std::fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(format!(
        "{}\n{}\n{modified}",
        path.display(),
        metadata.len()
    ))
}

fn verify_opentopia_sandbox_binary(path: &Path) -> anyhow::Result<()> {
    let fingerprint = sandbox_binary_fingerprint(path)?;
    let cache = WINDOWS_SANDBOX_PROTOCOL_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(result) = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(&fingerprint).cloned())
    {
        return result.map_err(anyhow::Error::msg);
    }

    let result = (|| -> anyhow::Result<()> {
        let output = Command::new(path)
            .args(["protocol", "--json"])
            .output()
            .map_err(|error| {
                anyhow::anyhow!(
                    "failed to start OpenTopia sandbox protocol handshake at '{}': {error}",
                    path.display()
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!(
                "OpenTopia sandbox helper at '{}' does not implement the required protocol handshake (exit {}): {}. Rebuild the server and helper as one runtime bundle.",
                path.display(),
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() { "no diagnostic" } else { &stderr }
            );
        }
        let info: opentopia_sandbox_protocol::SandboxProtocolInfo =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                anyhow::anyhow!(
                    "OpenTopia sandbox helper at '{}' returned an invalid protocol descriptor: {error}",
                    path.display()
                )
            })?;
        if let Some(error) = info.compatibility_error() {
            anyhow::bail!(
                "OpenTopia sandbox helper at '{}' is incompatible: {error}. Rebuild the server and helper as one runtime bundle.",
                path.display()
            );
        }
        Ok(())
    })()
    .map_err(|error| error.to_string());

    if let Ok(mut cache) = cache.lock() {
        cache.insert(fingerprint, result.clone());
    }
    result.map_err(anyhow::Error::msg)
}

fn resolve_opentopia_sandbox_binary() -> anyhow::Result<Option<PathBuf>> {
    let configured = std::env::var_os("OPENTOPIA_WINDOWS_SANDBOX_BIN").map(PathBuf::from);
    if let Some(configured) = configured {
        if !configured.is_file() {
            anyhow::bail!(
                "configured OpenTopia Windows sandbox helper was not found at '{}'",
                configured.display()
            );
        }
        verify_opentopia_sandbox_binary(&configured)?;
        return Ok(Some(configured));
    }
    let (sibling, cargo_debug_sibling) = std::env::current_exe()
        .ok()
        .map(|path| {
            let sibling = path
                .parent()
                .map(|parent| parent.join("opentopia-sandbox.exe"));
            // `cargo test` runs binaries from `target/<profile>/deps`; the
            // first-party helper is emitted one directory above it.
            let cargo_debug_sibling = path.parent().and_then(|parent| {
                parent
                    .parent()
                    .map(|target_profile| target_profile.join("opentopia-sandbox.exe"))
            });
            (sibling, cargo_debug_sibling)
        })
        .unwrap_or((None, None));
    let candidates = sibling
        .into_iter()
        .chain(cargo_debug_sibling)
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    let mut incompatibilities = Vec::new();
    for candidate in candidates {
        match verify_opentopia_sandbox_binary(&candidate) {
            Ok(()) => return Ok(Some(candidate)),
            Err(error) => incompatibilities.push(error.to_string()),
        }
    }
    if incompatibilities.is_empty() {
        Ok(None)
    } else {
        anyhow::bail!(incompatibilities.join("; "))
    }
}

pub fn windows_sandbox_setup_status() -> anyhow::Result<WindowsSandboxSetupStatus> {
    if OsSandboxPlatform::current() != OsSandboxPlatform::Windows {
        return Ok(WindowsSandboxSetupStatus {
            supported: false,
            helper_available: false,
            state: WindowsSandboxSetupState::Unavailable,
            backend: "dedicated_user".to_string(),
            state_dir: None,
            components: WindowsSandboxSetupComponents::default(),
            issues: vec!["the dedicated-user sandbox backend is available only on Windows".into()],
        });
    }
    let Some(helper) = resolve_opentopia_sandbox_binary()? else {
        return Ok(WindowsSandboxSetupStatus {
            supported: true,
            helper_available: false,
            state: WindowsSandboxSetupState::Unavailable,
            backend: "dedicated_user".to_string(),
            state_dir: None,
            components: WindowsSandboxSetupComponents::default(),
            issues: vec!["the OpenTopia Windows sandbox helper was not found".into()],
        });
    };
    let output = Command::new(&helper)
        .args(["setup", "--status", "--json"])
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to query Windows sandbox setup through '{}': {error}",
                helper.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows sandbox setup status failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let status: opentopia_sandbox_protocol::SandboxSetupStatus =
        serde_json::from_slice(&output.stdout).context("parse Windows sandbox setup status")?;
    if let Some(error) = status.compatibility_error() {
        anyhow::bail!("incompatible Windows sandbox setup status: {error}");
    }
    Ok(WindowsSandboxSetupStatus {
        supported: true,
        helper_available: true,
        state: match status.state {
            opentopia_sandbox_protocol::SandboxSetupState::NotConfigured => {
                WindowsSandboxSetupState::NotConfigured
            }
            opentopia_sandbox_protocol::SandboxSetupState::Ready => WindowsSandboxSetupState::Ready,
            opentopia_sandbox_protocol::SandboxSetupState::Degraded => {
                WindowsSandboxSetupState::Degraded
            }
        },
        backend: "dedicated_user".to_string(),
        state_dir: Some(status.state_dir),
        components: WindowsSandboxSetupComponents {
            credentials: status.components.credentials,
            offline_identity: status.components.offline_identity,
            online_identity: status.components.online_identity,
            offline_network_policy: status.components.offline_network_policy,
        },
        issues: status.issues,
    })
}

pub fn setup_windows_sandbox() -> anyhow::Result<WindowsSandboxSetupStatus> {
    run_windows_sandbox_lifecycle("setup", WindowsSandboxSetupState::Ready)
}

pub fn remove_windows_sandbox() -> anyhow::Result<WindowsSandboxSetupStatus> {
    run_windows_sandbox_lifecycle("teardown", WindowsSandboxSetupState::NotConfigured)
}

fn run_windows_sandbox_lifecycle(
    action: &str,
    expected_state: WindowsSandboxSetupState,
) -> anyhow::Result<WindowsSandboxSetupStatus> {
    anyhow::ensure!(
        OsSandboxPlatform::current() == OsSandboxPlatform::Windows,
        "the dedicated-user sandbox backend can be managed only on Windows"
    );
    let helper = resolve_opentopia_sandbox_binary()?
        .context("the OpenTopia Windows sandbox helper was not found")?;
    let output = Command::new(&helper)
        .arg(action)
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to start Windows sandbox {action} through '{}': {error}",
                helper.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows sandbox {action} failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let status = windows_sandbox_setup_status()?;
    anyhow::ensure!(
        status.state == expected_state,
        "Windows sandbox {action} exited successfully but reached state {:?}: {}",
        status.state,
        status.issues.join("; ")
    );
    Ok(status)
}

fn first_existing_executable(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

fn env_path_list(name: &str) -> Vec<PathBuf> {
    std::env::var_os(name)
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

fn default_system_read_paths() -> Vec<&'static str> {
    vec![
        "/bin", "/etc", "/lib", "/lib64", "/opt", "/usr", "/sbin", "/var",
    ]
}

fn seatbelt_profile(workspace_root: &Path, config: &LocalSandboxConfig) -> String {
    let workspace_root = absolute_path(workspace_root);
    let mut profile = vec![
        "(version 1)".to_string(),
        "(deny default)".to_string(),
        "(allow process*)".to_string(),
        "(allow signal (target self))".to_string(),
        "(allow sysctl-read)".to_string(),
        "(allow file-read-metadata)".to_string(),
        "(allow file-read*".to_string(),
        "  (subpath \"/bin\")".to_string(),
        "  (subpath \"/dev\")".to_string(),
        "  (subpath \"/etc\")".to_string(),
        "  (subpath \"/Library\")".to_string(),
        "  (subpath \"/System\")".to_string(),
        "  (subpath \"/usr\")".to_string(),
        format!("  (subpath \"{}\")", seatbelt_escape(&workspace_root)),
    ];

    for path in &config.read_paths {
        profile.push(format!(
            "  (subpath \"{}\")",
            seatbelt_escape(&absolute_path(path))
        ));
    }
    profile.push(")".to_string());

    if config.sandbox_mode == SandboxMode::WorkspaceWrite {
        profile.push("(allow file-write*".to_string());
        for path in config.effective_writable_roots(&workspace_root) {
            profile.push(format!(
                "  (subpath \"{}\")",
                seatbelt_escape(&absolute_path(&path))
            ));
        }
        profile.push("  (subpath \"/tmp\")".to_string());
        profile.push("  (subpath \"/private/tmp\")".to_string());
        profile.push(")".to_string());

        for path in protected_paths(&workspace_root, config) {
            profile.push(format!(
                "(deny file-write* (subpath \"{}\"))",
                seatbelt_escape(&absolute_path(path))
            ));
        }
    }

    if matches!(
        config.network,
        NetworkPolicy::Allow | NetworkPolicy::Inherit
    ) {
        profile.push("(allow network*)".to_string());
    }

    profile.join("\n")
}

fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn seatbelt_escape(path: &Path) -> String {
    path_to_string(path)
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn sandbox_capabilities(mode: SandboxMode) -> Vec<String> {
    let mut capabilities = vec![
        "read_file".to_string(),
        "search".to_string(),
        "shell".to_string(),
        "git_diff".to_string(),
        "spawn_stdio".to_string(),
        "os_sandbox_preflight".to_string(),
    ];
    if mode != SandboxMode::ReadOnly {
        capabilities.push("write_file".to_string());
        capabilities.push("apply_patch".to_string());
    }
    capabilities
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

fn protected_paths(workspace_root: &Path, config: &LocalSandboxConfig) -> Vec<PathBuf> {
    if config.sandbox_mode != SandboxMode::WorkspaceWrite {
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

fn dedup_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for path in paths {
        let path = absolute_path(path);
        if !result.iter().any(|existing| existing == &path) {
            result.push(path);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_args() -> Vec<String> {
        vec!["-lc".to_string(), "echo ok".to_string()]
    }

    #[test]
    fn local_sandbox_config_defaults_to_disabled() {
        let config = LocalSandboxConfig::default();
        assert!(!config.is_enabled());
        assert_eq!(config.mode, OsSandboxMode::Disabled);
        assert_eq!(config.network, NetworkPolicy::Deny);
        assert_eq!(config.sandbox_mode, SandboxMode::WorkspaceWrite);
    }

    #[test]
    fn dedicated_user_backend_keeps_the_legacy_elevated_setting_compatible() {
        assert_eq!(
            serde_json::from_str::<WindowsSandboxBackend>(r#""elevated""#).unwrap(),
            WindowsSandboxBackend::DedicatedUser
        );
        assert_eq!(
            serde_json::to_string(&WindowsSandboxBackend::DedicatedUser).unwrap(),
            r#""dedicated_user""#
        );
    }

    #[test]
    fn approved_missing_path_matches_its_canonical_parent_representation() {
        let root =
            std::env::temp_dir().join(format!("opentopia-approved-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create approved path fixture");
        let mut config = LocalSandboxConfig::default();
        config.grant_write_path(root.join(".codex/config.toml"));
        let canonical = root
            .canonicalize()
            .expect("canonicalize approved path fixture");

        assert!(config.is_approved_write_path(&canonical.join(".codex/config.toml")));
        assert!(!config.is_approved_write_path(&canonical.join(".codex/sibling.toml")));

        std::fs::remove_dir_all(root).expect("remove approved path fixture");
    }

    #[test]
    fn local_sandbox_config_deserializes_from_camel_case() {
        let config: LocalSandboxConfig = serde_json::from_str(
            r#"{
                "enabled": true,
                "mode": "best_effort",
                "network": "allow",
                "readPaths": ["C:/readonly"],
                "writePaths": ["C:/workspace"]
            }"#,
        )
        .expect("deserialize sandbox config");

        assert!(config.is_enabled());
        assert_eq!(config.mode, OsSandboxMode::BestEffort);
        assert_eq!(config.network, NetworkPolicy::Allow);
        assert_eq!(config.read_paths, vec![PathBuf::from("C:/readonly")]);
        assert_eq!(config.write_paths, vec![PathBuf::from("C:/workspace")]);
        assert_eq!(config.sandbox_mode, SandboxMode::WorkspaceWrite);
    }

    #[test]
    fn disabled_sandbox_plan_preserves_command() {
        let args = sample_args();
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Linux,
            "sh",
            &args,
            Path::new("/workspace"),
            Path::new("/workspace"),
            &LocalSandboxConfig::default(),
        )
        .expect("build plan");

        assert_eq!(plan.program, "sh");
        assert_eq!(plan.args, args);
        assert_eq!(plan.status, SandboxCommandStatus::Disabled);
    }

    #[test]
    fn danger_full_access_plan_is_explicitly_unrestricted() {
        let args = sample_args();
        let config = LocalSandboxConfig::enforce().with_sandbox_mode(SandboxMode::DangerFullAccess);
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Linux,
            "sh",
            &args,
            Path::new("/workspace"),
            Path::new("/workspace"),
            &config,
        )
        .expect("build unrestricted plan");

        assert_eq!(plan.program, "sh");
        assert_eq!(plan.args, args);
        assert_eq!(plan.status, SandboxCommandStatus::Unrestricted);
    }

    #[test]
    fn narrowing_a_tool_profile_does_not_enable_a_disabled_os_sandbox() {
        let config =
            LocalSandboxConfig::danger_full_access().with_sandbox_mode(SandboxMode::ReadOnly);
        assert_eq!(config.sandbox_mode, SandboxMode::ReadOnly);
        assert_eq!(config.mode, OsSandboxMode::Disabled);
        assert!(!config.is_enabled());
    }

    #[test]
    fn linux_sandbox_plan_wraps_with_bubblewrap() {
        let args = sample_args();
        let mut config = LocalSandboxConfig::best_effort();
        config.network = NetworkPolicy::Deny;
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Linux,
            "sh",
            &args,
            Path::new("/workspace"),
            Path::new("/workspace"),
            &config,
        )
        .expect("build plan");

        assert!(plan.program.ends_with("bwrap"));
        assert!(plan.args.contains(&"--unshare-net".to_string()));
        assert!(plan.args.contains(&"--bind".to_string()));
        assert_eq!(plan.args.last(), Some(&"echo ok".to_string()));
        assert!(matches!(
            plan.status,
            SandboxCommandStatus::Wrapped {
                platform: OsSandboxPlatform::Linux,
                ..
            }
        ));
    }

    #[test]
    fn linux_read_only_uses_only_read_only_workspace_bind() {
        let config = LocalSandboxConfig::enforce().with_sandbox_mode(SandboxMode::ReadOnly);
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Linux,
            "sh",
            &sample_args(),
            Path::new("/workspace"),
            Path::new("/workspace"),
            &config,
        )
        .expect("build read-only plan");

        let workspace = path_to_string(&absolute_path("/workspace"));
        assert!(!plan.args.iter().any(|arg| arg == "--bind"));
        assert!(plan.args.windows(3).any(|args| {
            args[0] == "--ro-bind" && args[1] == workspace && args[2] == workspace
        }));
    }

    #[test]
    fn linux_workspace_write_includes_additional_writable_roots() {
        let mut config = LocalSandboxConfig::enforce();
        config.writable_roots = vec![PathBuf::from("/shared")];
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Linux,
            "sh",
            &sample_args(),
            Path::new("/workspace"),
            Path::new("/workspace"),
            &config,
        )
        .expect("build workspace-write plan");

        let shared = path_to_string(&absolute_path("/shared"));
        assert!(plan
            .args
            .windows(3)
            .any(|args| { args[0] == "--bind" && args[1] == shared && args[2] == shared }));
    }

    #[test]
    fn macos_sandbox_plan_wraps_with_sandbox_exec() {
        let args = sample_args();
        let mut config = LocalSandboxConfig::best_effort();
        config.network = NetworkPolicy::Deny;
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Macos,
            "sh",
            &args,
            Path::new("/workspace"),
            Path::new("/workspace"),
            &config,
        )
        .expect("build plan");

        assert!(plan.program.ends_with("sandbox-exec"));
        assert_eq!(plan.args.first(), Some(&"-p".to_string()));
        assert!(plan.args[1].contains("(deny default)"));
        assert!(plan.args[1].contains("workspace"));
        assert!(!plan.args[1].contains("(allow network*)"));
    }

    #[test]
    fn macos_read_only_profile_has_no_write_grants() {
        let config = LocalSandboxConfig::enforce().with_sandbox_mode(SandboxMode::ReadOnly);
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Macos,
            "sh",
            &sample_args(),
            Path::new("/workspace"),
            Path::new("/workspace"),
            &config,
        )
        .expect("build read-only profile");

        assert!(!plan.args[1].contains("allow file-write"));
    }

    #[test]
    fn macos_workspace_profile_protects_agent_metadata() {
        let profile = seatbelt_profile(Path::new("/workspace"), &LocalSandboxConfig::enforce());
        let workspace = absolute_path("/workspace");
        assert!(profile.contains(&format!(
            "(deny file-write* (subpath \"{}\"))",
            seatbelt_escape(&workspace.join(".git"))
        )));
        assert!(profile.contains(&format!(
            "(deny file-write* (subpath \"{}\"))",
            seatbelt_escape(&workspace.join(".codex"))
        )));
    }

    #[test]
    fn windows_enforce_auto_selects_the_complete_dedicated_user_backend() {
        let root =
            std::env::temp_dir().join(format!("opentopia-windows-plan-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        let shared = root.join("shared");
        std::fs::create_dir_all(workspace.join(".git")).expect("create workspace");
        std::fs::create_dir_all(&shared).expect("create shared root");

        let mut config = LocalSandboxConfig::enforce();
        config.writable_roots = vec![shared.clone()];
        config.grant_read_path(root.join("read-only.txt"));
        let plan = build_windows_sandbox_command_with_binary(
            std::env::current_exe().expect("current executable"),
            "powershell.exe",
            &sample_args(),
            &workspace,
            &workspace,
            &config,
            &SandboxLaunchOptions {
                interactive: true,
                max_output_bytes: Some(65_536),
                ..SandboxLaunchOptions::default()
            },
        )
        .expect("build first-party Windows sandbox plan");

        let workspace_path = path_to_string(&absolute_path(&workspace));
        let shared_path = path_to_string(&absolute_path(&shared));
        assert_eq!(plan.args.first(), Some(&"run".to_string()));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args[0] == "--write-root" && args[1] == workspace_path));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args[0] == "--write-root" && args[1] == shared_path));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args[0] == "--protect" && args[1].ends_with(".git")));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args == ["--network", "deny"]));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args == ["--max-output-bytes", "65536"]));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args == ["--backend", "dedicated-user"]));
        assert!(plan.args.iter().any(|arg| arg == "--interactive"));
        assert!(plan
            .args
            .windows(2)
            .any(|args| args == ["--", "powershell.exe"]));
        assert!(matches!(
            plan.status,
            SandboxCommandStatus::Wrapped {
                platform: OsSandboxPlatform::Windows,
                ref backend,
            } if backend == "opentopia-windows-dedicated-user"
        ));
        assert!(plan
            .env
            .iter()
            .any(|(key, value)| key == "OPENTOPIA_SANDBOX_ERROR_NONCE" && !value.is_empty()));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn windows_best_effort_defers_backend_selection_to_provisioning_state() {
        let root =
            std::env::temp_dir().join(format!("opentopia-windows-plan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create workspace");
        let mut config = LocalSandboxConfig::best_effort();
        config.network = NetworkPolicy::Allow;

        let plan = build_windows_sandbox_command_with_binary(
            std::env::current_exe().expect("current executable"),
            "cmd.exe",
            &["/c".to_string(), "echo ok".to_string()],
            &root,
            &root,
            &config,
            &SandboxLaunchOptions::default(),
        )
        .expect("build best-effort Windows sandbox plan");

        assert!(plan
            .args
            .windows(2)
            .any(|args| args == ["--backend", "auto"]));
        assert!(matches!(
            plan.status,
            SandboxCommandStatus::Wrapped {
                platform: OsSandboxPlatform::Windows,
                ref backend,
            } if backend == "opentopia-windows-auto"
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn windows_enforce_rejects_the_partial_restricted_token_backend() {
        let root =
            std::env::temp_dir().join(format!("opentopia-windows-plan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create workspace");
        let mut config = LocalSandboxConfig::enforce();
        config.windows_backend = WindowsSandboxBackend::Unelevated;

        let error = build_windows_sandbox_command_with_binary(
            std::env::current_exe().expect("current executable"),
            "cmd.exe",
            &["/c".to_string(), "echo ok".to_string()],
            &root,
            &root,
            &config,
            &SandboxLaunchOptions::default(),
        )
        .expect_err("enforce mode must reject a partial backend");

        assert!(error.to_string().contains("arbitrary child-process IPC"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn windows_backend_capabilities_report_subprocess_ipc_truthfully() {
        assert!(
            SandboxBackendCapabilities::for_platform(
                OsSandboxPlatform::Windows,
                WindowsSandboxBackend::DedicatedUser,
            )
            .native_subprocess_ipc
        );
        assert!(
            !SandboxBackendCapabilities::for_platform(
                OsSandboxPlatform::Windows,
                WindowsSandboxBackend::Unelevated,
            )
            .native_subprocess_ipc
        );
    }

    #[test]
    fn windows_enforce_fails_closed_without_own_backend() {
        let result = unavailable_backend(
            OsSandboxPlatform::Windows,
            "OpenTopia Windows sandbox backend was not found",
            "powershell.exe",
            &sample_args(),
            &LocalSandboxConfig::enforce(),
        );
        assert!(result.is_err());
    }
}
