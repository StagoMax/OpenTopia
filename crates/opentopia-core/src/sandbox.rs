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
}

impl Default for LocalSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: OsSandboxMode::Disabled,
            network: NetworkPolicy::Deny,
            read_paths: Vec::new(),
            write_paths: Vec::new(),
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
            ..Self::default()
        }
    }

    pub fn enforce() -> Self {
        Self {
            enabled: true,
            mode: OsSandboxMode::Enforce,
            ..Self::default()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.mode != OsSandboxMode::Disabled
    }

    pub fn is_enforced(&self) -> bool {
        self.enabled && self.mode == OsSandboxMode::Enforce
    }

    pub fn from_env() -> Self {
        let mode = match std::env::var("OPENTOPIA_SANDBOX_MODE")
            .unwrap_or_else(|_| "disabled".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "enforce" | "strict" => OsSandboxMode::Enforce,
            "best_effort" | "best-effort" => OsSandboxMode::BestEffort,
            _ => OsSandboxMode::Disabled,
        };
        let network = match std::env::var("OPENTOPIA_SANDBOX_NETWORK")
            .unwrap_or_else(|_| "deny".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "allow" => NetworkPolicy::Allow,
            "inherit" => NetworkPolicy::Inherit,
            _ => NetworkPolicy::Deny,
        };
        Self {
            enabled: mode != OsSandboxMode::Disabled,
            mode,
            network,
            read_paths: env_path_list("OPENTOPIA_SANDBOX_READ_PATHS"),
            write_paths: env_path_list("OPENTOPIA_SANDBOX_WRITE_PATHS"),
        }
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
    pub backend: Option<String>,
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
                format!("OS sandbox is ready using {backend}."),
            ),
            Ok(SandboxCommandPlan {
                status: SandboxCommandStatus::BestEffortPassthrough { reason, .. },
                ..
            }) => (SandboxLifecycle::Ready, false, false, None, reason),
            Ok(_) => (
                SandboxLifecycle::Stopped,
                false,
                false,
                None,
                "OS sandbox is disabled by configuration.".to_string(),
            ),
            Err(err) => (SandboxLifecycle::Error, false, false, None, err.to_string()),
        };
        Self {
            id: format!("local-{thread_id}"),
            thread_id,
            kind: ExecutionEnvironmentKind::Local,
            lifecycle,
            workspace_root,
            capabilities: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "search".to_string(),
                "shell".to_string(),
                "git_diff".to_string(),
                "apply_patch".to_string(),
                "spawn_stdio".to_string(),
                "os_sandbox_preflight".to_string(),
            ],
            message,
            platform,
            mode: config.mode,
            network: config.network,
            backend,
            enforced,
            available,
        }
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

    for path in effective_read_paths(config, &workspace_root) {
        let path = absolute_path(&path);
        args.push("--ro-bind".to_string());
        args.push(path_to_string(&path));
        args.push(path_to_string(&path));
    }

    for path in effective_write_paths(config, &workspace_root) {
        let path = absolute_path(&path);
        args.push("--bind".to_string());
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
    if !config.read_paths.is_empty() || !config.write_paths.is_empty() {
        anyhow::bail!(
            "custom read/write path grants are not supported by the Windows Codex sandbox adapter"
        );
    }
    let Some(codex) = resolve_codex_sandbox_binary() else {
        return unavailable_backend(
            OsSandboxPlatform::Windows,
            "Codex restricted-token sandbox backend was not found",
            program,
            args,
            config,
        );
    };
    let codex_home = prepare_codex_sandbox_home(config)?;
    let mut sandbox_args = vec![
        "sandbox".to_string(),
        "--permission-profile".to_string(),
        "opentopia".to_string(),
        "--cd".to_string(),
        path_to_string(&absolute_path(cwd)),
    ];
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
            backend: "codex-restricted-token".to_string(),
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
    let home = std::env::var("OPENTOPIA_SANDBOX_HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            std::env::var("LOCALAPPDATA")
                .map(|root| PathBuf::from(root).join("OpenTopia").join("sandbox"))
        })
        .unwrap_or_else(|_| std::env::temp_dir().join("opentopia-sandbox"));
    std::fs::create_dir_all(&home)?;
    let network = if matches!(
        config.network,
        NetworkPolicy::Allow | NetworkPolicy::Inherit
    ) {
        "\n[permissions.opentopia.network]\nenabled = true\nmode = \"full\"\n"
    } else {
        ""
    };
    let contents = format!(
        "default_permissions = \"opentopia\"\n\n[permissions.opentopia]\nextends = \":workspace\"\n{network}"
    );
    let path = home.join("config.toml");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(contents.as_str()) {
        std::fs::write(&path, contents)?;
    }
    Ok(home)
}

fn first_existing_executable(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

fn env_path_list(name: &str) -> Vec<PathBuf> {
    std::env::var_os(name)
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default()
}

fn effective_read_paths(config: &LocalSandboxConfig, workspace_root: &Path) -> Vec<PathBuf> {
    if config.read_paths.is_empty() {
        Vec::new()
    } else {
        config.read_paths.clone()
    }
    .into_iter()
    .filter(|path| absolute_path(path) != absolute_path(workspace_root))
    .collect()
}

fn effective_write_paths(config: &LocalSandboxConfig, workspace_root: &Path) -> Vec<PathBuf> {
    if config.write_paths.is_empty() {
        vec![workspace_root.to_path_buf()]
    } else {
        config.write_paths.clone()
    }
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

    profile.push("(allow file-write*".to_string());
    for path in effective_write_paths(config, &workspace_root) {
        profile.push(format!(
            "  (subpath \"{}\")",
            seatbelt_escape(&absolute_path(&path))
        ));
    }
    profile.push("  (subpath \"/tmp\")".to_string());
    profile.push("  (subpath \"/private/tmp\")".to_string());
    profile.push(")".to_string());

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
    fn windows_adapter_rejects_unsupported_custom_path_grants() {
        let mut config = LocalSandboxConfig::enforce();
        config.write_paths = vec![PathBuf::from("C:/other")];
        let result = build_local_sandbox_command_for_platform(
            OsSandboxPlatform::Windows,
            "powershell.exe",
            &sample_args(),
            Path::new("C:/workspace"),
            Path::new("C:/workspace"),
            &config,
        );

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("custom read/write path grants"));
    }
}
