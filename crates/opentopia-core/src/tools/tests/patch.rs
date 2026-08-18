use super::*;

#[test]
fn plan_tools_describe_memory_and_evidence_without_mandating_a_scheduler() {
    assert!(Tool::description(&SetPlanTool).contains("external memory"));
    assert!(Tool::description(&UpdatePlanTool).contains("advisory"));
    assert!(!Tool::description(&UpdatePlanTool).contains("one step at a time"));
}

#[tokio::test]
async fn native_patch_operations_create_update_and_delete_one_target() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-native-patch-{}", Uuid::new_v4()));
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

    ApplyPatchTool
        .execute(
            ToolCall::new(
                "apply_patch",
                json!({
                    "operation": {
                        "type": "create_file",
                        "path": "notes.txt",
                        "diff": "@@ -0,0 +1,2 @@\n+hello\n+world\n"
                    }
                }),
            ),
            context.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(workspace_root.join("notes.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "hello\nworld\n"
    );

    execute_native_patch_operation(
        Uuid::new_v4(),
        NativePatchOperation::UpdateFile {
            path: "notes.txt".to_string(),
            diff: "@@ -1,2 +1,2 @@\n hello\n-world\n+earth\n".to_string(),
        },
        context.clone(),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read_to_string(workspace_root.join("notes.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "hello\nearth\n"
    );

    let mut approved_delete_context = context;
    approved_delete_context.approval_granted = true;
    execute_native_patch_operation(
        Uuid::new_v4(),
        NativePatchOperation::DeleteFile {
            path: "notes.txt".to_string(),
        },
        approved_delete_context,
    )
    .await
    .unwrap();
    assert!(!workspace_root.join("notes.txt").exists());
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn apply_patch_delete_requires_approval_even_in_full_access() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-delete-approval-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::write(workspace_root.join("delete-me.txt"), "fixture\n").unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );

    let error = execute_native_patch_operation(
        Uuid::new_v4(),
        NativePatchOperation::DeleteFile {
            path: "delete-me.txt".to_string(),
        },
        context.clone(),
    )
    .await
    .unwrap_err();
    assert!(crate::policy::approval_required(&error).is_some());
    assert!(workspace_root.join("delete-me.txt").exists());

    let mut approved = context;
    approved.approval_granted = true;
    execute_native_patch_operation(
        Uuid::new_v4(),
        NativePatchOperation::DeleteFile {
            path: "delete-me.txt".to_string(),
        },
        approved,
    )
    .await
    .unwrap();
    assert!(!workspace_root.join("delete-me.txt").exists());
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn apply_patch_accepts_codex_envelopes_and_search_replace_updates() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-patch-envelope-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    fs::write(
        workspace_root.join("styles.css"),
        ".composer {\n  border: 1px solid gray;\n  background: white;\n}\n",
    )
    .unwrap();
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );

    let envelope = "*** Begin Patch\n*** Update File: styles.css\n@@\n .composer {\n-  border: 1px solid gray;\n+  border: 0;\n   background: white;\n }\n*** End Patch";
    let result = execute_portable_patch(Uuid::new_v4(), envelope, context.clone())
        .await
        .unwrap();
    assert_eq!(result.metadata["changedPaths"], json!(["styles.css"]));
    assert!(fs::read_to_string(workspace_root.join("styles.css"))
        .unwrap()
        .contains("border: 0"));

    execute_native_patch_operation(
        Uuid::new_v4(),
        NativePatchOperation::UpdateFile {
            path: "styles.css".to_string(),
            diff: "<<<<<<< SEARCH\n  border: 0;\n=======\n  border: none;\n>>>>>>> REPLACE"
                .to_string(),
        },
        context,
    )
    .await
    .unwrap();
    assert!(fs::read_to_string(workspace_root.join("styles.css"))
        .unwrap()
        .contains("border: none"));
    fs::remove_dir_all(workspace_root).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn apply_patch_created_unicode_powershell_script_runs_in_windows_powershell_51() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-powershell-encoding-{}", Uuid::new_v4()));
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
    let envelope = "*** Begin Patch\n*** Add File: unicode.ps1\n+$utf8 = New-Object System.Text.UTF8Encoding($false)\n+[Console]::OutputEncoding = $utf8\n+Write-Output \"中文（全角括号）\"\n*** End Patch";

    execute_portable_patch(Uuid::new_v4(), envelope, context.clone())
        .await
        .unwrap();
    let script_path = workspace_root.join("unicode.ps1");
    let bytes = fs::read(&script_path).unwrap();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));

    let script = script_path.to_string_lossy().into_owned();
    let output = context
        .environment
        .exec(
            ExecRequest::new("powershell.exe").args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                script.as_str(),
            ]),
            ExecutionContext::with_timeout(Duration::from_secs(20)),
        )
        .await
        .unwrap();
    assert!(
        output.success,
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "中文（全角括号）"
    );
    fs::remove_dir_all(workspace_root).unwrap();
}

#[test]
fn unified_text_patch_uses_context_when_provider_line_numbers_are_stale() {
    let original = "header\n.composer {\n  border: 1px solid gray;\n  background: white;\n}\n";
    let diff = "@@ -3500,4 +3500,4 @@\n .composer {\n-  border: 1px solid gray;\n+  border: 0;\n   background: white;\n }\n";
    let updated = apply_text_patch(original, diff).unwrap();
    assert!(updated.contains("border: 0"));
    assert!(!updated.contains("border: 1px"));
}

#[test]
fn native_patch_rejects_path_injection_and_retargets_full_diffs() {
    let error = native_patch_operation_to_unified_diff(&NativePatchOperation::UpdateFile {
        path: "../escape.txt".to_string(),
        diff: "@@ -1 +1 @@\n-old\n+new\n".to_string(),
    })
    .unwrap_err();
    assert!(error.to_string().contains("cannot contain '..'"));

    let patch = native_patch_operation_to_unified_diff(&NativePatchOperation::UpdateFile {
        path: "safe.txt".to_string(),
        diff: "--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-old\n+new\n".to_string(),
    })
    .unwrap();
    assert!(patch.contains("--- a/safe.txt"));
    assert!(!patch.contains("other.txt"));
}
