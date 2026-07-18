use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

    pub fn with_sandbox_mode(mut self, sandbox_mode: SandboxMode) -> Self {
        self.sandbox_mode = sandbox_mode;
        if sandbox_mode == SandboxMode::DangerFullAccess {
            self.enabled = false;
            self.mode = OsSandboxMode::Disabled;
            self.network = NetworkPolicy::Allow;
        } else if self.mode == OsSandboxMode::Disabled {
            self.enabled = true;
            self.mode = OsSandboxMode::Enforce;
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
            .unwrap_or_else(|_| {
                if sandbox_mode == SandboxMode::DangerFullAccess {
                    "allow".to_string()
                } else {
                    "deny".to_string()
                }
            })
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
    build_local_sandbox_command_for_platform(
        OsSandboxPlatform::current(),
        program,
        args,
        cwd,
        workspace_root,
        config,
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
            build_windows_sandbox_command(program, args, cwd, workspace_root, config)
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
) -> anyhow::Result<SandboxCommandPlan> {
    let Some(codex) = resolve_codex_sandbox_binary() else {
        let reason = std::env::var("OPENTOPIA_SANDBOX_BACKEND_ERROR")
            .unwrap_or_else(|_| "Codex restricted-token sandbox backend was not found".to_string());
        return unavailable_backend(OsSandboxPlatform::Windows, reason, program, args, config);
    };
    let permission_profile = windows_permission_profile(config);
    let codex_home = prepare_codex_sandbox_home(config)?;
    let mut sandbox_args = vec!["sandbox".to_string()];
    for config_override in windows_permission_overrides(workspace_root, config) {
        sandbox_args.push("--config".to_string());
        sandbox_args.push(config_override);
    }
    sandbox_args.extend([
        "--permission-profile".to_string(),
        permission_profile.to_string(),
        "--cd".to_string(),
        path_to_string(&absolute_path(cwd)),
    ]);
    sandbox_args.push("--".to_string());
    sandbox_args.push(program.to_string());
    sandbox_args.extend(args.iter().cloned());

    Ok(SandboxCommandPlan {
        program: path_to_string(&codex),
        args: sandbox_args,
        env: vec![
            ("CODEX_HOME".to_string(), path_to_string(&codex_home)),
            (
                "OPENTOPIA_SANDBOX_WORKSPACE".to_string(),
                path_to_string(&absolute_path(workspace_root)),
            ),
        ],
        status: SandboxCommandStatus::Wrapped {
            platform: OsSandboxPlatform::Windows,
            backend: format!(
                "codex-windows-{}",
                windows_sandbox_implementation().replace('_', "-")
            ),
        },
    })
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

fn resolve_codex_sandbox_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OPENTOPIA_CODEX_SANDBOX_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
        if std::env::var("OPENTOPIA_REQUIRE_CODEX_SANDBOX_BIN").as_deref() == Ok("true") {
            return None;
        }
    } else if std::env::var("OPENTOPIA_REQUIRE_CODEX_SANDBOX_BIN").as_deref() == Ok("true") {
        return None;
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let candidate = PathBuf::from(profile)
            .join(".codex")
            .join("plugins")
            .join(".plugin-appserver")
            .join("codex.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn prepare_codex_sandbox_home(config: &LocalSandboxConfig) -> anyhow::Result<PathBuf> {
    let sandbox_root = config
        .sandbox_home
        .clone()
        .or_else(|| {
            std::env::var("OPENTOPIA_SANDBOX_HOME")
                .ok()
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::var("LOCALAPPDATA")
                .map(|root| PathBuf::from(root).join("OpenTopia").join("sandbox"))
                .ok()
        })
        .unwrap_or_else(|| std::env::temp_dir().join("opentopia-sandbox"));
    let implementation = windows_sandbox_implementation();
    let contents = [
        "[windows]".to_string(),
        format!("sandbox = \"{implementation}\""),
        "sandbox_private_desktop = true".to_string(),
    ]
    .join("\n");
    let home = sandbox_root.join(format!("codex-home-v2-{implementation}"));
    std::fs::create_dir_all(&home)?;
    let path = home.join("config.toml");
    if path.exists() {
        anyhow::ensure!(
            std::fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()),
            "sandbox base profile content is inconsistent"
        );
    } else {
        let temporary = home.join(format!("config-{}.tmp", Uuid::new_v4()));
        std::fs::write(&temporary, &contents)?;
        if let Err(error) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            if !path.exists() {
                return Err(error.into());
            }
        }
        anyhow::ensure!(
            std::fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()),
            "sandbox base profile content is inconsistent"
        );
    }
    Ok(home)
}

fn windows_permission_overrides(workspace_root: &Path, config: &LocalSandboxConfig) -> Vec<String> {
    if windows_permission_profile(config) != "opentopia" {
        return Vec::new();
    }

    let base = match config.sandbox_mode {
        SandboxMode::ReadOnly => ":read-only",
        SandboxMode::WorkspaceWrite => ":workspace",
        SandboxMode::DangerFullAccess => ":danger-full-access",
    };
    let mut overrides = vec![format!(
        "permissions.opentopia.extends={}",
        toml_basic_string(base)
    )];
    let filesystem_entries = config
        .read_paths
        .iter()
        .chain(config.approved_read_paths.iter())
        .map(|path| (absolute_path(path), "read"))
        .chain(
            config
                .effective_writable_roots(workspace_root)
                .into_iter()
                .filter(|path| absolute_path(path) != absolute_path(workspace_root))
                .map(|path| (absolute_path(path), "write")),
        )
        .map(|(path, access)| {
            format!(
                "{}={}",
                toml_basic_string(&path_to_string(&path)),
                toml_basic_string(access)
            )
        })
        .collect::<Vec<_>>();
    if !filesystem_entries.is_empty() {
        overrides.push(format!(
            "permissions.opentopia.filesystem={{{}}}",
            filesystem_entries.join(",")
        ));
    }
    if matches!(
        config.network,
        NetworkPolicy::Allow | NetworkPolicy::Inherit
    ) {
        overrides.extend([
            "permissions.opentopia.network.enabled=true".to_string(),
            "permissions.opentopia.network.mode=\"full\"".to_string(),
        ]);
    }
    overrides
}

fn windows_permission_profile(config: &LocalSandboxConfig) -> &'static str {
    let has_extra_roots = !config.read_paths.is_empty()
        || !config.write_paths.is_empty()
        || !config.writable_roots.is_empty()
        || !config.approved_read_paths.is_empty()
        || !config.approved_write_paths.is_empty();
    let network_enabled = matches!(
        config.network,
        NetworkPolicy::Allow | NetworkPolicy::Inherit
    );
    if has_extra_roots || network_enabled {
        "opentopia"
    } else {
        match config.sandbox_mode {
            SandboxMode::ReadOnly => ":read-only",
            SandboxMode::WorkspaceWrite => ":workspace",
            SandboxMode::DangerFullAccess => ":danger-full-access",
        }
    }
}

fn windows_sandbox_implementation() -> &'static str {
    match std::env::var("OPENTOPIA_WINDOWS_SANDBOX")
        .unwrap_or_else(|_| "unelevated".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "elevated" => "elevated",
        _ => "unelevated",
    }
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

fn toml_basic_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
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
    fn linux_sandbox_plan_wraps_with_bubblewrap() {
        let args = sample_args();
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Linux,
            "sh",
            &args,
            Path::new("/workspace"),
            Path::new("/workspace"),
            &LocalSandboxConfig::best_effort(),
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
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Macos,
            "sh",
            &args,
            Path::new("/workspace"),
            Path::new("/workspace"),
            &LocalSandboxConfig::best_effort(),
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
    fn windows_best_effort_uses_restricted_token_backend_when_available() {
        let args = sample_args();
        let plan = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Windows,
            "powershell.exe",
            &args,
            Path::new("C:/workspace"),
            Path::new("C:/workspace"),
            &LocalSandboxConfig::best_effort(),
        )
        .expect("build plan");

        if resolve_codex_sandbox_binary().is_some() {
            assert!(matches!(
                plan.status,
                SandboxCommandStatus::Wrapped {
                    platform: OsSandboxPlatform::Windows,
                    ..
                }
            ));
            assert!(plan.args.iter().any(|arg| arg == "sandbox"));
            assert_eq!(plan.args.last(), Some(&"echo ok".to_string()));
        } else {
            assert!(matches!(
                plan.status,
                SandboxCommandStatus::BestEffortPassthrough {
                    platform: OsSandboxPlatform::Windows,
                    ..
                }
            ));
        }
    }

    #[test]
    fn windows_enforce_uses_restricted_token_backend() {
        let args = sample_args();
        let result = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Windows,
            "powershell.exe",
            &args,
            Path::new("C:/workspace"),
            Path::new("C:/workspace"),
            &LocalSandboxConfig::enforce(),
        );

        if resolve_codex_sandbox_binary().is_some() {
            let plan = result.expect("windows enforce should use Codex sandbox backend");
            assert!(matches!(
                plan.status,
                SandboxCommandStatus::Wrapped {
                    platform: OsSandboxPlatform::Windows,
                    ..
                }
            ));
        } else {
            assert!(result.is_err());
        }
    }

    #[test]
    fn windows_adapter_passes_custom_path_grants_per_invocation() {
        let sandbox_home = std::env::temp_dir().join(format!(
            "opentopia-sandbox-profile-{}",
            uuid::Uuid::new_v4()
        ));
        let mut config = LocalSandboxConfig::enforce();
        config.sandbox_home = Some(sandbox_home.clone());
        config.writable_roots = vec![PathBuf::from("C:/other")];
        let result = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Windows,
            "powershell.exe",
            &sample_args(),
            Path::new("C:/workspace"),
            Path::new("C:/workspace"),
            &config,
        );

        if resolve_codex_sandbox_binary().is_some() {
            let plan = result.expect("custom roots should use a generated Codex profile");
            assert!(plan
                .args
                .windows(2)
                .any(|args| { args == ["--permission-profile", "opentopia"] }));
            let codex_home = plan
                .env
                .iter()
                .find_map(|(key, value)| (key == "CODEX_HOME").then_some(value))
                .expect("plan includes CODEX_HOME");
            let contents = std::fs::read_to_string(Path::new(codex_home).join("config.toml"))
                .expect("read base profile");
            assert!(!contents.contains("[permissions.opentopia]"));
            let overrides = plan
                .args
                .windows(2)
                .filter(|args| args[0] == "--config")
                .map(|args| args[1].as_str())
                .collect::<Vec<_>>();
            assert!(overrides
                .iter()
                .any(|value| value == &"permissions.opentopia.extends=\":workspace\""));
            assert!(overrides.iter().any(|value| value.contains("C:/other")));
        } else {
            assert!(result.is_err());
        }
        let _ = std::fs::remove_dir_all(sandbox_home);
    }

    #[test]
    fn windows_permission_overrides_are_isolated_by_effective_permissions() {
        let workspace = std::env::temp_dir().join(format!(
            "opentopia-sandbox-profile-workspace-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace).expect("create workspace");
        let mut first = LocalSandboxConfig::enforce();
        first.grant_read_path(workspace.join("first.txt"));
        let mut second = LocalSandboxConfig::enforce();
        second.grant_read_path(workspace.join("second.txt"));

        let first_overrides = windows_permission_overrides(&workspace, &first).join("\n");
        let second_overrides = windows_permission_overrides(&workspace, &second).join("\n");
        assert_ne!(first_overrides, second_overrides);
        assert!(first_overrides.contains("first.txt"));
        assert!(!first_overrides.contains("second.txt"));
        assert!(second_overrides.contains("second.txt"));
        assert!(!second_overrides.contains("first.txt"));

        let _ = std::fs::remove_dir_all(workspace);
    }
}
