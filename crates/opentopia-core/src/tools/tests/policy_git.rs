use super::*;

#[test]
fn tool_execution_policy_marks_observations_as_parallel_safe() {
    let registry = ToolRegistry::with_core_tools();
    let read = ToolCall::new(
        "filesystem",
        json!({ "operation": "read", "path": "src/lib.rs" }),
    );
    let policy = registry.execution_policy("filesystem", &read).unwrap();
    assert!(policy.read_only);
    assert!(policy.idempotent);
    assert!(policy.parallel_safe);
    assert_eq!(policy.side_effect, ToolSideEffect::None);
    assert_eq!(policy.resource_keys, vec!["file:src/lib.rs"]);

    let shell = ToolCall::new("shell", json!({ "command": "git status" }));
    let policy = registry.execution_policy("shell", &shell).unwrap();
    assert!(policy.read_only);
    assert!(policy.idempotent);
    assert!(policy.parallel_safe);
    assert_eq!(policy.side_effect, ToolSideEffect::None);
    assert!(policy.resource_keys.is_empty());

    let dynamic_shell = ToolCall::new("shell", json!({ "command": "cargo test" }));
    let policy = registry.execution_policy("shell", &dynamic_shell).unwrap();
    assert!(!policy.read_only);
    assert!(!policy.idempotent);
    assert_eq!(policy.side_effect, ToolSideEffect::Process);

    let background_read = ToolCall::new(
        "shell",
        json!({ "command": "git status", "background": true }),
    );
    let policy = registry
        .execution_policy("shell", &background_read)
        .unwrap();
    assert!(!policy.read_only);
    assert_eq!(policy.side_effect, ToolSideEffect::Process);
}

#[test]
fn structured_observation_and_control_tools_declare_scoped_parallelism() {
    let registry = ToolRegistry::with_builtins();
    let document_open = registry
        .execution_policy(
            "document_open",
            &ToolCall::new(
                "document_open",
                json!({
                    "resource": { "kind": "file", "path": "reports/a.xlsx" },
                    "mode": "read"
                }),
            ),
        )
        .unwrap();
    assert!(document_open.read_only);
    assert!(document_open.parallel_safe);
    assert_eq!(document_open.resource_keys, vec!["file:reports/a.xlsx"]);

    let list_skills =
        <ListSkillsTool as TypedTool>::execution_policy(&ListSkillsTool, &EmptyToolInput {});
    assert!(list_skills.read_only);
    assert_eq!(list_skills.resource_keys, vec!["skills:catalog"]);

    let read_skill = <ReadSkillTool as TypedTool>::execution_policy(
        &ReadSkillTool,
        &ReadSkillInput {
            id: "system/test".to_string(),
            offset: 0,
            limit: None,
        },
    );
    assert!(read_skill.read_only);
    assert_eq!(read_skill.resource_keys, vec!["skill:system/test"]);

    let list_agents = <ListAgentsTool as TypedTool>::execution_policy(
        &ListAgentsTool,
        &ListAgentsInput { path_prefix: None },
    );
    assert!(list_agents.read_only);
    assert_eq!(list_agents.resource_keys, vec!["agents:tree"]);

    let attachment = <ViewAttachmentTool as TypedTool>::execution_policy(
        &ViewAttachmentTool,
        &ViewAttachmentInput {
            attachment_id: Uuid::new_v4().to_string(),
            focus: None,
        },
    );
    assert!(attachment.read_only);
    assert!(attachment.parallel_safe);

    let job_id = Uuid::new_v4().to_string();
    let background_read = <BackgroundOutputTool as TypedTool>::execution_policy(
        &BackgroundOutputTool,
        &BackgroundOutputInput::Read {
            job_id: job_id.clone(),
            timeout_ms: None,
        },
    );
    assert!(!background_read.read_only);
    assert!(background_read.parallel_safe);
    assert_eq!(
        background_read.resource_keys,
        vec![format!("session:{job_id}")]
    );

    let send_agent = <SendAgentMessageTool as TypedTool>::execution_policy(
        &SendAgentMessageTool,
        &AgentTargetMessageInput {
            target: "/root/reviewer".to_string(),
            message: "check".to_string(),
        },
    );
    assert!(send_agent.parallel_safe);
    assert_eq!(send_agent.resource_keys, vec!["agent:/root/reviewer"]);

    let isolated_spawn = <SpawnAgentTool as TypedTool>::execution_policy(
        &SpawnAgentTool,
        &SpawnAgentInput {
            task_name: "reviewer".to_string(),
            message: "check".to_string(),
            fork_turns: None,
            agent_type: "default".to_string(),
            workspace_mode: AgentWorkspaceModeInput::IsolatedWorktree,
            allow_child_spawns: false,
        },
    );
    assert!(isolated_spawn.parallel_safe);
    assert_eq!(isolated_spawn.resource_keys, vec!["git:index-and-worktree"]);
}

#[tokio::test]
async fn git_diff_returns_worktree_changes_through_the_execution_environment() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-git-diff-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace_root)
        .status()
        .unwrap();
    assert!(init.success());
    fs::write(workspace_root.join("sample.txt"), "before\n").unwrap();
    let add = std::process::Command::new("git")
        .args(["add", "--", "sample.txt"])
        .current_dir(&workspace_root)
        .status()
        .unwrap();
    assert!(add.success());
    fs::write(workspace_root.join("sample.txt"), "after\n").unwrap();

    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );
    let result = GitDiffTool
        .execute(ToolCall::new("git_diff", json!({})), context)
        .await
        .unwrap();

    assert_eq!(result.metadata["success"], true);
    assert!(result.output.contains("-before"));
    assert!(result.output.contains("+after"));
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn portable_patch_process_uses_backend_compatible_workspace_intent() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-git-apply-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace_root)
        .status()
        .unwrap();
    assert!(init.success());
    fs::write(workspace_root.join("sample.txt"), "before\n").unwrap();

    let policy = Arc::new(BasicPolicyEngine::new(
        workspace_root.clone(),
        PermissionMode::FullAccess,
    ));
    let context = ToolInvocationContext::local_with_sandbox_config(
        workspace_root.clone(),
        policy,
        LocalSandboxConfig::danger_full_access(),
    );
    let result = ApplyPatchTool
        .execute(
            ToolCall::new(
                "apply_patch",
                json!({
                    "patch": "--- a/sample.txt\n+++ b/sample.txt\n@@ -1 +1 @@\n-before\n+after\n"
                }),
            ),
            context,
        )
        .await
        .unwrap();

    assert_eq!(result.metadata["success"], true);
    assert_eq!(
        fs::read_to_string(workspace_root.join("sample.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "after\n"
    );
    fs::remove_dir_all(workspace_root).unwrap();
}

#[tokio::test]
async fn common_git_read_commands_execute_through_the_model_shell() {
    let workspace_root =
        std::env::temp_dir().join(format!("opentopia-git-read-matrix-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace_root).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&workspace_root)
        .status()
        .unwrap();
    assert!(init.success());
    fs::write(workspace_root.join("sample.txt"), "fixture\n").unwrap();
    let add = std::process::Command::new("git")
        .args(["add", "--", "sample.txt"])
        .current_dir(&workspace_root)
        .status()
        .unwrap();
    assert!(add.success());
    let commit = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=OpenTopia Test",
            "-c",
            "user.email=opentopia@example.invalid",
            "commit",
            "--quiet",
            "--message",
            "fixture commit",
        ])
        .current_dir(&workspace_root)
        .status()
        .unwrap();
    assert!(commit.success());

    let commands = [
        "git status --short --branch",
        "git log --oneline -1",
        "git log -L 1,1:sample.txt --oneline -1",
        "git show --stat --oneline HEAD",
        "git rev-parse --show-toplevel",
        "git branch --list",
        "git worktree list --porcelain",
        "git blame -L 1,1 -- sample.txt",
        "git ls-files -- sample.txt",
        "git diff --no-ext-diff --no-color --",
    ];
    let command = if cfg!(windows) {
        commands
            .iter()
            .map(|command| format!("{command}; if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}"))
            .collect::<Vec<_>>()
            .join("; ")
    } else {
        format!("set -e; {}", commands.join("; "))
    };
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
            ToolCall::new("shell", json!({ "command": command })),
            context,
        )
        .await
        .unwrap();

    assert_eq!(result.metadata["success"], true, "{}", result.output);
    assert!(result.output.contains("fixture commit"));
    assert!(result.output.contains("sample.txt"));
    fs::remove_dir_all(workspace_root).unwrap();
}
