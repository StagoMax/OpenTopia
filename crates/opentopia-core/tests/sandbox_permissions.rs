use opentopia_core::{
    build_local_sandbox_command_with_options, BasicPolicyEngine, ExecutionEnvironment,
    ExecutionGrant, FileWriteRequest, LocalExecutionEnvironment, LocalSandboxConfig,
    PermissionMode, PolicyDecision, PolicyEngine, ProcessLifetime, SandboxLaunchOptions,
    ToolExecutionIntent,
};
use uuid::Uuid;

fn lease_fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let id = Uuid::new_v4();
    let workspace = std::env::temp_dir().join(format!("opentopia-{name}-workspace-{id}"));
    let outside = std::env::temp_dir().join(format!("opentopia-{name}-outside-{id}"));
    std::fs::create_dir_all(&workspace).expect("create lease workspace");
    std::fs::create_dir_all(&outside).expect("create lease outside root");
    (workspace, outside)
}

#[test]
fn ordinary_reads_are_global_but_exact_write_approval_does_not_authorize_siblings() {
    let (workspace, outside) = lease_fixture("policy-lease");
    let approved_read = outside.join("approved-read.txt");
    let approved_write = outside.join("approved-write.txt");
    let sibling = outside.join("sibling.txt");
    std::fs::write(&approved_read, "approved").expect("create approved read fixture");

    let mut sandbox = LocalSandboxConfig::default();
    sandbox.grant_read_path(approved_read.clone());
    sandbox.grant_write_path(approved_write.clone());
    let policy = BasicPolicyEngine::new_with_sandbox_config(
        workspace.clone(),
        PermissionMode::Auto,
        &sandbox,
    );

    assert!(matches!(
        policy.inspect_read(&approved_read),
        PolicyDecision::Allow
    ));
    assert!(matches!(
        policy.inspect_write(&approved_write),
        PolicyDecision::Allow
    ));
    assert!(matches!(
        policy.inspect_read(&sibling),
        PolicyDecision::Allow
    ));
    assert!(matches!(
        policy.inspect_write(&sibling),
        PolicyDecision::Ask { .. }
    ));

    std::fs::remove_dir_all(workspace).expect("remove lease workspace");
    std::fs::remove_dir_all(outside).expect("remove lease outside root");
}

#[tokio::test]
async fn exact_external_write_lease_is_enforced_by_the_execution_environment() {
    let (workspace, outside) = lease_fixture("execution-lease");
    let approved = outside.join("approved.txt");
    let sibling = outside.join("sibling.txt");
    let mut sandbox = LocalSandboxConfig::default();
    sandbox.grant_write_path(approved.clone());
    let environment = LocalExecutionEnvironment::with_sandbox_config(workspace.clone(), sandbox);

    environment
        .write_file(FileWriteRequest::new(&approved, b"allowed".to_vec()))
        .await
        .expect("write exact approved path");
    let error = environment
        .write_file(FileWriteRequest::new(&sibling, b"blocked".to_vec()))
        .await
        .expect_err("sibling must remain outside the lease");

    assert!(error.to_string().contains("escapes workspace"));
    assert_eq!(std::fs::read_to_string(approved).unwrap(), "allowed");
    assert!(!sibling.exists());
    std::fs::remove_dir_all(workspace).expect("remove lease workspace");
    std::fs::remove_dir_all(outside).expect("remove lease outside root");
}

#[test]
fn exact_file_lease_is_not_inherited_by_shell_intent() {
    let (workspace, outside) = lease_fixture("shell-projection");
    let approved = outside.join("approved.txt");
    let mut sandbox = LocalSandboxConfig::enforce();
    sandbox.grant_write_path(approved);

    let grant = ExecutionGrant::resolve(
        &sandbox,
        &workspace,
        &ToolExecutionIntent::session_process(ProcessLifetime::OneShot),
        false,
    )
    .expect("resolve shell execution grant");

    assert!(grant.sandbox.approved_read_paths.is_empty());
    assert!(grant.sandbox.approved_write_paths.is_empty());
    std::fs::remove_dir_all(workspace).expect("remove shell projection workspace");
    std::fs::remove_dir_all(outside).expect("remove shell projection outside root");
}

#[cfg(windows)]
#[test]
fn workspace_runtime_directory_is_not_reclassified_as_external() {
    let (workspace, outside) = lease_fixture("workspace-runtime");
    let runtime = workspace.join("node_modules").join(".bin");
    std::fs::create_dir_all(&runtime).expect("create workspace runtime");
    let plan = build_local_sandbox_command_with_options(
        "node.exe",
        &[],
        &workspace,
        &workspace,
        &LocalSandboxConfig::enforce(),
        &SandboxLaunchOptions {
            runtime_read_roots: vec![runtime.clone()],
            ..SandboxLaunchOptions::default()
        },
    )
    .expect("build Windows sandbox plan");

    let comparable = |path: &str| {
        path.replace('/', "\\")
            .trim_start_matches(r"\\?\")
            .to_ascii_lowercase()
    };
    let expected_runtime = comparable(&runtime.to_string_lossy());
    assert!(!plan
        .args
        .windows(2)
        .any(|args| { args[0] == "--runtime-root" && comparable(&args[1]) == expected_runtime }));

    std::fs::remove_dir_all(workspace).expect("remove runtime workspace");
    std::fs::remove_dir_all(outside).expect("remove unused outside fixture");
}
