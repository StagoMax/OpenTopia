//! Cross-platform sandbox command preparation and backend selection.

use super::contract::{
    LocalSandboxConfig, NetworkPolicy, OsSandboxMode, OsSandboxPlatform, SandboxCommandPlan,
    SandboxCommandStatus, SandboxLaunchOptions, SandboxMode, SandboxPreparationPlan,
    WindowsSandboxBackend,
};
use super::path_policy::{
    absolute_path, dedup_paths, path_to_string, protected_paths, seatbelt_escape,
    windows_path_starts_with,
};
use super::windows_backend::resolve_opentopia_sandbox_binary;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub fn sandbox_permission_profile(
    platform: OsSandboxPlatform,
    config: &LocalSandboxConfig,
) -> String {
    match platform {
        OsSandboxPlatform::Windows => windows_permission_profile(config).to_string(),
        _ => config.sandbox_mode.as_str().to_string(),
    }
}

pub(super) fn sandbox_probe_command(platform: OsSandboxPlatform) -> (String, Vec<String>) {
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
        // Codex-style restricted modes expose the host filesystem read-only,
        // then overlay only the configured writable roots below.
        "--ro-bind".to_string(),
        "/".to_string(),
        "/".to_string(),
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
        preparation: None,
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
        preparation: None,
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

pub(super) fn build_windows_sandbox_command_with_binary(
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
    // Restricted modes intentionally allow host reads. Runtime roots resolved
    // from PATH/SDK metadata therefore belong to the managed read capability,
    // too: the dedicated Windows account may need an explicit RX ACE even when
    // the interactive user can already execute them. Keep only the fixed OS
    // runtime roots immutable (`--runtime-root`).
    let immutable_runtime_roots = windows_minimal_runtime_roots()
        .map(|root| absolute_path(root))
        .collect::<Vec<_>>();
    let configured_managed_roots = config.effective_command_readable_roots(&workspace_root);
    let managed_read_roots = dedup_paths(
        configured_managed_roots.iter().cloned().chain(
            options
                .runtime_read_roots
                .iter()
                .filter(|runtime| {
                    !immutable_runtime_roots
                        .iter()
                        .any(|root| windows_path_starts_with(runtime, root))
                        && !configured_managed_roots
                            .iter()
                            .any(|root| windows_path_starts_with(runtime, root))
                })
                .cloned(),
        ),
    )
    .into_iter()
    .filter(|root| root.exists())
    .map(|root| absolute_path(root))
    .collect::<Vec<_>>();
    for root in &managed_read_roots {
        sandbox_args.extend(["--read-root".to_string(), path_to_string(root)]);
    }
    for root in immutable_runtime_roots
        .into_iter()
        .filter(|root| root.exists())
        .filter(|runtime| {
            !managed_read_roots
                .iter()
                .any(|managed| windows_path_starts_with(runtime, managed))
        })
    {
        sandbox_args.extend(["--runtime-root".to_string(), path_to_string(&root)]);
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
    let mut preparation_args = sandbox_args.clone();
    preparation_args[0] = "provision".to_string();
    let preparation_key = preparation_args.join("\u{0}");
    sandbox_args.push("--".to_string());
    sandbox_args.push(program.to_string());
    sandbox_args.extend(args.iter().cloned());

    let sandbox_program = path_to_string(&sandbox);
    let env = {
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
    };
    Ok(SandboxCommandPlan {
        program: sandbox_program.clone(),
        args: sandbox_args,
        env: env.clone(),
        preparation: Some(SandboxPreparationPlan {
            key: preparation_key,
            program: sandbox_program,
            args: preparation_args,
            env,
        }),
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
            preparation: None,
            status: SandboxCommandStatus::BestEffortPassthrough { platform, reason },
        }),
        OsSandboxMode::Enforce => anyhow::bail!("{reason}"),
    }
}

pub(super) fn unavailable_backend(
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
        preparation: None,
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

fn first_existing_executable(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

pub(super) fn seatbelt_profile(workspace_root: &Path, config: &LocalSandboxConfig) -> String {
    let workspace_root = absolute_path(workspace_root);
    let mut profile = vec![
        "(version 1)".to_string(),
        "(deny default)".to_string(),
        "(allow process*)".to_string(),
        "(allow signal (target self))".to_string(),
        "(allow sysctl-read)".to_string(),
        "(allow file-read-metadata)".to_string(),
        "(allow file-read*)".to_string(),
    ];

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
