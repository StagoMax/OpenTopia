use super::command::{
    build_windows_sandbox_command_with_binary, seatbelt_profile, unavailable_backend,
};
use super::path_policy::{
    absolute_path, path_to_string, seatbelt_escape, windows_comparison_path,
    windows_path_starts_with,
};
use super::{
    build_local_sandbox_command_for_platform, LocalSandboxConfig, NetworkPolicy, OsSandboxMode,
    OsSandboxPlatform, SandboxBackendCapabilities, SandboxCommandStatus, SandboxLaunchOptions,
    SandboxMode, WindowsSandboxBackend,
};
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
fn sandbox_modes_only_attenuate_toward_read_only() {
    assert!(SandboxMode::ReadOnly.is_attenuation_of(SandboxMode::WorkspaceWrite));
    assert!(SandboxMode::WorkspaceWrite.is_attenuation_of(SandboxMode::DangerFullAccess));
    assert!(!SandboxMode::DangerFullAccess.is_attenuation_of(SandboxMode::WorkspaceWrite));
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
    let config = LocalSandboxConfig::danger_full_access().with_sandbox_mode(SandboxMode::ReadOnly);
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
fn linux_read_only_exposes_host_as_read_only_without_write_binds() {
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

    assert!(!plan.args.iter().any(|arg| arg == "--bind"));
    assert!(plan
        .args
        .windows(3)
        .any(|args| { args[0] == "--ro-bind" && args[1] == "/" && args[2] == "/" }));
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

    assert!(plan.args[1].contains("(allow file-read*)"));
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
    let runtime_home = root.join("runtime-home");
    std::fs::create_dir_all(workspace.join(".git")).expect("create workspace");
    std::fs::create_dir_all(&shared).expect("create shared root");
    std::fs::create_dir_all(&runtime_home).expect("create runtime home");
    let external_read = root.join("read-only.txt");
    std::fs::write(&external_read, "readable").expect("create external read fixture");

    let mut config = LocalSandboxConfig::enforce();
    config.sandbox_home = Some(runtime_home.clone());
    config.writable_roots = vec![shared.clone()];
    config.grant_read_path(external_read.clone());
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
    assert!(plan.args.windows(2).any(|args| {
        args[0] == "--read-root" && args[1] == path_to_string(&absolute_path(&external_read))
    }));
    assert!(plan.args.windows(2).any(|args| {
        args[0] == "--runtime-home" && args[1] == path_to_string(&absolute_path(&runtime_home))
    }));
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
    let baseline = plan
        .baseline_preparation
        .as_ref()
        .expect("Windows sandbox plan has an account baseline phase");
    assert_eq!(
        baseline.args.first().map(String::as_str),
        Some("provision-baseline")
    );
    let preparation = plan
        .preparation
        .as_ref()
        .expect("Windows sandbox plan has an explicit ACL preparation phase");
    assert_eq!(
        preparation.args.first().map(String::as_str),
        Some("provision-scope")
    );
    assert!(!preparation.args.iter().any(|arg| arg == "powershell.exe"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn windows_account_baseline_cache_is_shared_across_filesystem_scopes() {
    let root = std::env::temp_dir().join(format!(
        "opentopia-windows-baseline-plan-{}",
        Uuid::new_v4()
    ));
    let first_workspace = root.join("first");
    let second_workspace = root.join("second");
    std::fs::create_dir_all(&first_workspace).expect("create first workspace");
    std::fs::create_dir_all(&second_workspace).expect("create second workspace");
    let build = |workspace: &Path, config: &LocalSandboxConfig| {
        build_windows_sandbox_command_with_binary(
            std::env::current_exe().expect("current executable"),
            "cmd.exe",
            &["/c".to_string(), "exit 0".to_string()],
            workspace,
            workspace,
            config,
            &SandboxLaunchOptions::default(),
        )
        .expect("build Windows sandbox plan")
    };

    let offline = LocalSandboxConfig::enforce();
    let first = build(&first_workspace, &offline);
    let second = build(&second_workspace, &offline);
    assert_eq!(
        first
            .baseline_preparation
            .as_ref()
            .expect("first baseline")
            .key,
        second
            .baseline_preparation
            .as_ref()
            .expect("second baseline")
            .key
    );
    assert_ne!(
        first.preparation.as_ref().expect("first scope").key,
        second.preparation.as_ref().expect("second scope").key
    );

    let mut online = LocalSandboxConfig::enforce();
    online.network = NetworkPolicy::Allow;
    let online_plan = build(&first_workspace, &online);
    assert_ne!(
        first.baseline_preparation.expect("offline baseline").key,
        online_plan
            .baseline_preparation
            .expect("online baseline")
            .key
    );
    std::fs::remove_dir_all(root).expect("remove baseline plan fixture");
}

#[test]
fn windows_runtime_inside_managed_workspace_is_not_classified_external() {
    let root = std::env::temp_dir().join(format!("opentopia-runtime-plan-{}", Uuid::new_v4()));
    let runtime = root.join("node_modules").join(".bin");
    std::fs::create_dir_all(&runtime).expect("create workspace runtime");
    let plan = build_windows_sandbox_command_with_binary(
        std::env::current_exe().expect("current executable"),
        "node.exe",
        &sample_args(),
        &root,
        &root,
        &LocalSandboxConfig::enforce(),
        &SandboxLaunchOptions {
            runtime_read_roots: vec![runtime.clone()],
            ..Default::default()
        },
    )
    .expect("build Windows sandbox plan");
    assert!(!plan.args.windows(2).any(|args| {
        args[0] == "--runtime-root" && windows_path_starts_with(Path::new(&args[1]), &runtime)
    }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn windows_external_runtime_is_provisioned_as_a_managed_read_root() {
    let root = std::env::temp_dir().join(format!(
        "opentopia-external-runtime-plan-{}",
        Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let runtime = root.join("user-runtime");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&runtime).expect("create external runtime");

    let plan = build_windows_sandbox_command_with_binary(
        std::env::current_exe().expect("current executable"),
        "runtime.exe",
        &sample_args(),
        &workspace,
        &workspace,
        &LocalSandboxConfig::enforce(),
        &SandboxLaunchOptions {
            runtime_read_roots: vec![runtime.clone()],
            ..Default::default()
        },
    )
    .expect("build Windows sandbox plan");

    assert!(plan.args.windows(2).any(|args| {
        args[0] == "--read-root" && windows_path_starts_with(Path::new(&args[1]), &runtime)
    }));
    assert!(!plan.args.windows(2).any(|args| {
        args[0] == "--runtime-root" && windows_path_starts_with(Path::new(&args[1]), &runtime)
    }));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn windows_managed_runtime_parent_is_projected_separately_from_generation() {
    let root =
        std::env::temp_dir().join(format!("opentopia-managed-runtime-plan-{}", Uuid::new_v4()));
    let workspace = root.join("workspace");
    let runtime_root = root.join("runtimes").join("pnpm");
    let generation = runtime_root.join("10.30.0");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&generation).expect("create runtime generation");

    let plan = build_windows_sandbox_command_with_binary(
        std::env::current_exe().expect("current executable"),
        "node.exe",
        &sample_args(),
        &workspace,
        &workspace,
        &LocalSandboxConfig::enforce(),
        &SandboxLaunchOptions {
            runtime_read_roots: vec![generation.clone()],
            managed_runtime_roots: vec![runtime_root.clone()],
            ..Default::default()
        },
    )
    .expect("build managed runtime sandbox plan");

    assert!(plan.args.windows(2).any(|args| {
        args[0] == "--read-root" && windows_path_starts_with(Path::new(&args[1]), &generation)
    }));
    assert!(plan.args.windows(2).any(|args| {
        args[0] == "--managed-runtime-root"
            && windows_path_starts_with(Path::new(&args[1]), &runtime_root)
    }));
    let preparation = plan.preparation.expect("managed runtime preparation");
    assert!(preparation
        .args
        .iter()
        .any(|arg| arg == "--managed-runtime-root"));
    std::fs::remove_dir_all(root).expect("remove managed runtime plan fixture");
}

#[test]
fn windows_preparation_cache_invalidates_when_runtime_generation_is_replaced() {
    let root = std::env::temp_dir().join(format!(
        "opentopia-runtime-generation-plan-{}",
        Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    let runtime_root = root.join("runtimes");
    let generation = runtime_root.join("current");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&generation).expect("create first runtime generation");
    let options = SandboxLaunchOptions {
        runtime_read_roots: vec![generation.clone()],
        managed_runtime_roots: vec![runtime_root],
        ..Default::default()
    };
    let build = || {
        build_windows_sandbox_command_with_binary(
            std::env::current_exe().expect("current executable"),
            "node.exe",
            &sample_args(),
            &workspace,
            &workspace,
            &LocalSandboxConfig::enforce(),
            &options,
        )
        .expect("build runtime generation plan")
        .preparation
        .expect("runtime generation preparation")
        .key
    };

    let first_key = build();
    std::fs::remove_dir_all(&generation).expect("remove first runtime generation");
    std::fs::create_dir_all(&generation).expect("publish replacement runtime generation");
    let second_key = build();
    assert_ne!(first_key, second_key);
    std::fs::remove_dir_all(root).expect("remove generation cache fixture");
}

#[cfg(windows)]
#[test]
fn windows_path_containment_normalizes_verbatim_namespaces() {
    let native_root = Path::new(r"J:\Project\OpenTopia");
    let verbatim_root = Path::new(r"\\?\J:\Project\OpenTopia");
    let native_runtime = Path::new(r"J:\Project\OpenTopia\apps\desktop\node_modules\.bin");
    let verbatim_runtime = Path::new(r"\\?\J:\Project\OpenTopia\apps\desktop\node_modules\.bin");

    assert!(windows_path_starts_with(native_runtime, verbatim_root));
    assert!(windows_path_starts_with(verbatim_runtime, native_root));
    assert!(!windows_path_starts_with(
        Path::new(r"J:\Project\OpenTopia2\node_modules\.bin"),
        verbatim_root,
    ));
}

#[cfg(windows)]
#[test]
fn windows_native_runtime_inside_verbatim_workspace_is_managed() {
    let root = std::env::temp_dir().join(format!("opentopia-runtime-plan-{}", Uuid::new_v4()));
    let runtime = root.join("node_modules").join(".bin");
    std::fs::create_dir_all(&runtime).expect("create workspace runtime");
    let verbatim_root = root.canonicalize().expect("canonical workspace root");
    let native_runtime = PathBuf::from(windows_comparison_path(&runtime));

    assert!(verbatim_root.to_string_lossy().starts_with(r"\\?\"));
    assert!(!native_runtime.to_string_lossy().starts_with(r"\\?\"));

    let plan = build_windows_sandbox_command_with_binary(
        std::env::current_exe().expect("current executable"),
        "node.exe",
        &sample_args(),
        &verbatim_root,
        &verbatim_root,
        &LocalSandboxConfig::enforce(),
        &SandboxLaunchOptions {
            runtime_read_roots: vec![native_runtime.clone()],
            ..Default::default()
        },
    )
    .expect("build Windows sandbox plan");

    assert!(!plan.args.windows(2).any(|args| {
        args[0] == "--runtime-root"
            && windows_path_starts_with(Path::new(&args[1]), &native_runtime)
    }));
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
fn windows_enforce_uses_explicit_streaming_backend_for_persistent_stdio() {
    let root =
        std::env::temp_dir().join(format!("opentopia-windows-plan-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create workspace");
    let mut config = LocalSandboxConfig::enforce();
    config.network = NetworkPolicy::Allow;

    let plan = build_windows_sandbox_command_with_binary(
        std::env::current_exe().expect("current executable"),
        "cmd.exe",
        &["/c".to_string(), "echo ok".to_string()],
        &root,
        &root,
        &config,
        &SandboxLaunchOptions {
            persistent_stdio: true,
            ..Default::default()
        },
    )
    .expect("persistent stdio should select the streaming sandbox backend");

    assert!(plan.args.iter().any(|arg| arg == "--persistent-stdio"));
    assert!(plan
        .args
        .windows(2)
        .any(|args| args == ["--backend", "unelevated"]));
    assert!(matches!(
        plan.status,
        SandboxCommandStatus::Wrapped {
            platform: OsSandboxPlatform::Windows,
            ref backend,
        } if backend == "opentopia-windows-restricted-token"
    ));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn windows_persistent_stdio_does_not_downgrade_offline_networking() {
    let root =
        std::env::temp_dir().join(format!("opentopia-windows-plan-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create workspace");

    let error = build_windows_sandbox_command_with_binary(
        std::env::current_exe().expect("current executable"),
        "cmd.exe",
        &["/c".to_string(), "echo ok".to_string()],
        &root,
        &root,
        &LocalSandboxConfig::enforce(),
        &SandboxLaunchOptions {
            persistent_stdio: true,
            ..Default::default()
        },
    )
    .expect_err("offline persistent stdio must fail closed");

    assert!(error
        .to_string()
        .contains("cannot authoritatively enforce offline networking"));
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
    assert!(
        !SandboxBackendCapabilities::for_platform(
            OsSandboxPlatform::Windows,
            WindowsSandboxBackend::DedicatedUser,
        )
        .recursive_write_allowlist
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
