use super::*;
use crate::NetworkAccess;

#[cfg(windows)]
#[tokio::test]
async fn write_file_allows_verbatim_workspace_target_in_approve_mode() {
    let workspace_root = std::env::temp_dir().join(format!(
        "opentopia-write-verbatim-workspace-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(workspace_root.join("design")).expect("create workspace fixture");
    let verbatim_root = workspace_root.canonicalize().expect("canonical workspace");
    assert!(verbatim_root.to_string_lossy().starts_with(r"\\?\"));
    let target = verbatim_root.join("design/requirements.md");
    let policy = Arc::new(BasicPolicyEngine::new(
        verbatim_root.clone(),
        PermissionMode::Approve,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        verbatim_root,
        policy,
        LocalSandboxConfig::default(),
    );

    let result = WriteFileTool
        .execute(
            ToolCall::new(
                "write_file",
                json!({
                    "path": target.display().to_string(),
                    "content": "workspace write is authorized"
                }),
            ),
            context,
        )
        .await
        .expect("workspace write must not require approval");

    assert_eq!(result.metadata["changedPath"], target.display().to_string());
    assert_eq!(
        fs::read_to_string(&target).expect("read written fixture"),
        "workspace write is authorized"
    );
    fs::remove_dir_all(workspace_root).expect("remove workspace fixture");
}

#[tokio::test]
async fn full_access_write_file_keeps_exact_external_path_capability() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-full-access-workspace-{id}"));
    let outside = std::env::temp_dir().join(format!("opentopia-full-access-outside-{id}"));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let target = outside.join("result.txt");
    let sandbox = LocalSandboxConfig::danger_full_access();
    let policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
        workspace_root.clone(),
        PermissionMode::FullAccess,
        &sandbox,
    ));
    let context =
        ToolInvocationContext::local_with_sandbox_config(workspace_root.clone(), policy, sandbox);

    WriteFileTool
        .execute(
            ToolCall::new(
                "write_file",
                json!({ "path": target.display().to_string(), "content": "allowed" }),
            ),
            context.clone(),
        )
        .await
        .expect("full-access session must preserve exact external write capability");

    assert_eq!(fs::read_to_string(&target).unwrap(), "allowed");
    let read = ReadFileTool
        .execute(
            ToolCall::new("read_file", json!({ "path": target.display().to_string() })),
            context,
        )
        .await
        .expect("full-access session must preserve exact external read capability");
    assert_eq!(read.output, "allowed");
    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[tokio::test]
async fn apply_patch_external_path_requires_and_honors_exact_approval() {
    let id = Uuid::new_v4();
    let workspace_root = std::env::temp_dir().join(format!("opentopia-patch-workspace-{id}"));
    let outside = std::env::temp_dir().join(format!("opentopia-patch-outside-{id}"));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let target = outside.join("approved.txt");
    let sibling = outside.join("sibling.txt");
    fs::write(&target, "before\n").unwrap();
    fs::write(&sibling, "sibling before\n").unwrap();
    let patch_for = |path: &Path, before: &str, after: &str| {
        format!(
            "*** Begin Patch\n*** Update File: {}\n@@\n-{before}\n+{after}\n*** End Patch",
            path.display()
        )
    };

    let base_sandbox = LocalSandboxConfig::default();
    let base_policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
        workspace_root.clone(),
        PermissionMode::Approve,
        &base_sandbox,
    ));
    let error = ApplyPatchTool
        .execute(
            ToolCall::new(
                "apply_patch",
                json!({ "patch": patch_for(&target, "before", "after") }),
            ),
            ToolInvocationContext::local_with_sandbox_config(
                workspace_root.clone(),
                base_policy,
                base_sandbox,
            ),
        )
        .await
        .expect_err("external patch must request approval");
    assert!(crate::policy::approval_required(&error).is_some());
    assert_eq!(fs::read_to_string(&target).unwrap(), "before\n");

    let mut approved_sandbox = LocalSandboxConfig::default();
    approved_sandbox.grant_write_path(target.clone());
    let approved_policy = Arc::new(BasicPolicyEngine::new_with_sandbox_config(
        workspace_root.clone(),
        PermissionMode::Approve,
        &approved_sandbox,
    ));
    let mut approved = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        approved_policy,
        approved_sandbox,
    );
    approved.approval_granted = true;
    ApplyPatchTool
        .execute(
            ToolCall::new(
                "apply_patch",
                json!({ "patch": patch_for(&target, "before", "after") }),
            ),
            approved.clone(),
        )
        .await
        .expect("approved external patch");
    assert_eq!(fs::read_to_string(&target).unwrap(), "after\n");

    let sibling_error = ApplyPatchTool
        .execute(
            ToolCall::new(
                "apply_patch",
                json!({ "patch": patch_for(&sibling, "sibling before", "sibling after") }),
            ),
            approved,
        )
        .await
        .expect_err("exact target approval must not authorize a sibling");
    assert!(format!("{sibling_error:#}").contains("escapes workspace"));
    assert_eq!(fs::read_to_string(&sibling).unwrap(), "sibling before\n");

    fs::remove_dir_all(workspace_root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn truncation_preserves_diagnostic_head_and_tail() {
    let value = format!("HEAD{}TAIL", "x".repeat(100));
    let truncated = truncate(&value, 20);
    assert!(truncated.starts_with("HEAD"));
    assert!(truncated.ends_with("TAIL"));
    assert!(truncated.contains("characters omitted"));

    let (bytes, was_truncated) = truncate_bytes(&value, 20);
    assert!(was_truncated);
    assert!(bytes.starts_with("HEAD"));
    assert!(bytes.ends_with("TAIL"));
    assert!(bytes.contains("bytes omitted"));
}

#[tokio::test]
async fn read_files_reads_multiple_windows_in_one_call() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-read-files-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::write(workspace_root.join("a.txt"), "zero\nalpha\nomega\n").unwrap();
    fs::write(workspace_root.join("b.txt"), "bravo").unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local(workspace_root.clone(), policy);

    let result = ReadFilesTool
        .execute(
            ToolCall::new(
                "read_files",
                json!({
                    "files": [
                        { "path": "a.txt", "startLine": 2, "endLine": 2 },
                        { "path": "b.txt" }
                    ]
                }),
            ),
            context,
        )
        .await
        .unwrap();
    assert!(result.output.contains("alpha"));
    assert!(!result.output.contains("omega"));
    assert!(result.output.contains("bravo"));
    assert_eq!(result.metadata["succeeded"], 2);
    fs::remove_dir_all(workspace_root).unwrap();
}

#[test]
fn shell_intent_projects_known_external_reads_through_unknown_pipeline_segments() {
    let requested = PathBuf::from("C:\\Users\\me\\Downloads\\orders.csv");
    let sibling = PathBuf::from("C:\\Users\\me\\Downloads\\other.csv");
    let analysis = analyze_shell_command(
            "Import-Csv -LiteralPath 'C:\\Users\\me\\Downloads\\orders.csv' | Where-Object Status -eq open",
        );
    let intent = shell_execution_intent(&analysis);

    assert_eq!(intent.filesystem, FilesystemAccess::ReadWorkspace);
    assert_eq!(
        intent.approval_escalation,
        ApprovalEscalation::CommandScopedHostAccess
    );
    assert_eq!(intent.requested_read_paths, vec![requested.clone()]);

    let grant = ExecutionGrant::resolve(
        &LocalSandboxConfig::enforce(),
        Path::new("C:\\workspace"),
        &intent,
        false,
    )
    .unwrap();
    assert_eq!(grant.sandbox.sandbox_mode, SandboxMode::ReadOnly);
    assert!(grant.sandbox.is_within_approved_read_scope(&requested));
    assert!(!grant.sandbox.is_within_approved_read_scope(&sibling));
    assert!(grant.sandbox.approved_write_paths.is_empty());
}

#[test]
fn shell_intent_requests_network_for_nested_powershell_calls() {
    let analysis = analyze_shell_command(
        "$uri = 'https://example.test'; try { Invoke-WebRequest -Uri $uri } catch { exit 1 }",
    );
    let intent = shell_execution_intent(&analysis);

    assert_eq!(intent.network, NetworkAccess::Required);
}

#[tokio::test]
async fn shell_honors_workspace_relative_workdir() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-shell-workdir-{}", Uuid::new_v4()));
    fs::create_dir_all(workspace_root.join("nested")).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );
    let command = if cfg!(windows) {
        "(Get-Location).Path"
    } else {
        "pwd"
    };
    let result = ShellTool
        .execute(
            ToolCall::new("shell", json!({ "command": command, "workdir": "nested" })),
            context,
        )
        .await
        .unwrap();
    assert!(result.output.contains("nested"));
    assert!(result.metadata["workdir"]
        .as_str()
        .is_some_and(|path| path.ends_with("nested")));
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn shell_automatically_yields_a_slow_command_to_the_existing_registry() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-shell-yield-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let mut context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );
    context.thread_id = Some(Uuid::new_v4());
    context.background = Some(BackgroundProcessRegistry::default());
    context.minimum_foreground_yield = Duration::from_millis(10);
    let scope = background_scope(&context).unwrap();
    let registry = context.background.clone().unwrap();
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };

    let result = ShellTool
        .execute(
            ToolCall::new("shell", json!({ "command": command, "yieldTimeMs": 10 })),
            context,
        )
        .await
        .unwrap();

    assert_eq!(result.metadata["background"], true);
    assert_eq!(result.metadata["autoDetached"], true);
    assert_eq!(result.metadata["yieldTimeMs"], 10);
    let job_id = Uuid::parse_str(result.metadata["jobId"].as_str().unwrap()).unwrap();
    assert_eq!(registry.list(&scope).len(), 1);
    registry.stop(&scope, job_id).unwrap();
    for _ in 0..100 {
        if registry
            .list(&scope)
            .iter()
            .all(|job| job.status.is_terminal())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    fs::remove_dir_all(workspace_root).unwrap();
}

#[test]
fn shell_cannot_shorten_the_runtime_foreground_window() {
    assert_eq!(
        effective_foreground_yield_milliseconds(
            Some(1_000),
            Duration::from_millis(DEFAULT_FOREGROUND_YIELD_MILLISECONDS),
        ),
        DEFAULT_FOREGROUND_YIELD_MILLISECONDS
    );
    assert_eq!(
        effective_foreground_yield_milliseconds(
            Some(60_000),
            Duration::from_millis(DEFAULT_FOREGROUND_YIELD_MILLISECONDS),
        ),
        60_000
    );
}

#[tokio::test]
async fn shell_keeps_a_quick_registered_command_in_the_foreground() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-shell-inline-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let mut context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );
    context.thread_id = Some(Uuid::new_v4());
    context.background = Some(BackgroundProcessRegistry::default());
    let command = if cfg!(windows) {
        "Write-Output inline-ready"
    } else {
        "echo inline-ready"
    };

    let result = ShellTool
        .execute(
            ToolCall::new("shell", json!({ "command": command, "yieldTimeMs": 10000 })),
            context,
        )
        .await
        .unwrap();

    assert!(result.output.contains("inline-ready"));
    assert_eq!(result.metadata["success"], true);
    assert!(result.metadata.get("background").is_none());
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn shell_rejects_unreviewable_destructive_target_before_execution() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-shell-reviewability-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );

    let result = ShellTool
        .execute(
            ToolCall::new("shell", json!({ "command": "rm -rf $target" })),
            context,
        )
        .await
        .unwrap();

    assert_eq!(result.metadata["success"], false);
    assert_eq!(result.metadata["reviewability"], "unreviewable_action");
    assert_eq!(
        result.metadata["errorRecord"]["code"],
        "unreviewable_action"
    );
    assert_eq!(result.metadata["errorRecord"]["executed"], false);
    assert_eq!(result.metadata["errorRecord"]["retryable"], true);
    fs::remove_dir_all(workspace_root).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn windows_powershell_51_rejects_unsupported_connectors_before_execution() {
    if ShellDialect::current() != ShellDialect::WindowsPowerShell51 {
        return;
    }
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-shell-dialect-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );

    let result = ShellTool
        .execute(
            ToolCall::new(
                "shell",
                json!({
                    "command": "git status && git log -1 | head -20"
                }),
            ),
            context,
        )
        .await
        .unwrap();

    assert_eq!(result.metadata["success"], false);
    assert_eq!(
        result.metadata["shellDialect"],
        ShellDialect::WindowsPowerShell51.id()
    );
    assert_eq!(
        result.metadata["errorRecord"]["code"],
        "shell_dialect_mismatch"
    );
    assert_eq!(result.metadata["errorRecord"]["executed"], false);
    assert_eq!(result.metadata["errorRecord"]["retryable"], true);
    assert!(result.output.contains("Select-Object -First/-Last"));
    fs::remove_dir_all(workspace_root).unwrap();
}
