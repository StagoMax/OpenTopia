#![cfg(windows)]

use std::net::TcpListener;
use std::process::Command;
use uuid::Uuid;

#[test]
fn restricted_token_can_write_only_to_granted_workspace() {
    let root = std::env::temp_dir().join(format!("opentopia-sandbox-test-{}", Uuid::new_v4()));
    let workspace = root.join("workspace");
    let inside = workspace.join("inside.txt");
    let outside = root.join("outside.txt");
    let protected_directory = workspace.join(".git");
    let protected = protected_directory.join("config");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&protected_directory).expect("create protected directory");
    std::fs::write(&protected, "original").expect("create existing protected file");

    let allowed_command = format!("echo inside>{}", inside.display());
    let allowed = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .args([
            "run",
            "--cwd",
            workspace.to_str().expect("workspace utf-8"),
            "--read-root",
            workspace.to_str().expect("workspace utf-8"),
            "--write-root",
            workspace.to_str().expect("workspace utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--",
            "cmd.exe",
            "/d",
            "/c",
            &allowed_command,
        ])
        .output()
        .expect("start allowed sandbox");

    assert!(
        allowed.status.success(),
        "granted workspace write should complete: status={:?} stdout={} stderr={}",
        allowed.status.code(),
        String::from_utf8_lossy(&allowed.stdout),
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert!(inside.is_file(), "granted workspace write should succeed");
    let denied_command = format!("echo outside>{}", outside.display());
    let denied = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .args([
            "run",
            "--cwd",
            workspace.to_str().expect("workspace utf-8"),
            "--read-root",
            workspace.to_str().expect("workspace utf-8"),
            "--write-root",
            workspace.to_str().expect("workspace utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--",
            "cmd.exe",
            "/d",
            "/c",
            &denied_command,
        ])
        .status()
        .expect("start denied sandbox");

    assert!(!denied.success(), "outside workspace write must fail");
    assert!(
        !outside.exists(),
        "sandbox must not write beside its granted workspace"
    );

    let denied_protected_command = format!("echo protected>{}", protected.display());
    let denied_protected = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .args([
            "run",
            "--cwd",
            workspace.to_str().expect("workspace utf-8"),
            "--read-root",
            workspace.to_str().expect("workspace utf-8"),
            "--write-root",
            workspace.to_str().expect("workspace utf-8"),
            "--protect",
            protected_directory
                .to_str()
                .expect("protected directory utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--",
            "cmd.exe",
            "/d",
            "/c",
            &denied_protected_command,
        ])
        .status()
        .expect("start protected-path sandbox");

    assert!(
        !denied_protected.success(),
        "protected metadata write must fail"
    );
    assert!(
        std::fs::read_to_string(&protected).expect("read protected file") == "original",
        "sandbox must preserve existing protected metadata"
    );
    std::fs::remove_dir_all(root).expect("remove sandbox fixture");
}

#[test]
fn unelevated_backend_rejects_an_offline_guarantee_it_cannot_enforce() {
    let root = std::env::temp_dir().join(format!("opentopia-network-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create workspace");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let port = listener.local_addr().expect("listener address").port();
    let command = format!(
        "$ErrorActionPreference='Stop'; $client = New-Object System.Net.Sockets.TcpClient; $client.Connect('127.0.0.1', {port}); exit 0"
    );

    let result = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .args([
            "run",
            "--cwd",
            root.to_str().expect("workspace utf-8"),
            "--read-root",
            root.to_str().expect("workspace utf-8"),
            "--write-root",
            root.to_str().expect("workspace utf-8"),
            "--network",
            "deny",
            "--backend",
            "unelevated",
            "--",
            "powershell.exe",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &command,
        ])
        .output()
        .expect("start network-denied sandbox");

    assert!(
        !result.status.success(),
        "unelevated backend silently accepted an offline guarantee: stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("cannot authoritatively enforce offline"),
        "missing capability error: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    drop(listener);
    std::fs::remove_dir_all(root).expect("remove network fixture");
}

#[test]
fn elevated_state_cannot_be_exposed_to_a_host_identity_child() {
    let root = std::env::temp_dir().join(format!("opentopia-credential-test-{}", Uuid::new_v4()));
    let state = root.join("state");
    std::fs::create_dir_all(&state).expect("create state fixture");
    std::fs::write(state.join("credentials.dpapi"), b"opaque").expect("create credential marker");

    let result = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .env("OPENTOPIA_SANDBOX_STATE_DIR", &state)
        .env("OPENTOPIA_SANDBOX_ERROR_NONCE", "credential-test")
        .args([
            "run",
            "--cwd",
            root.to_str().expect("workspace utf-8"),
            "--read-root",
            root.to_str().expect("workspace utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--",
            "cmd.exe",
            "/d",
            "/c",
            "exit 0",
        ])
        .output()
        .expect("run credential isolation test");

    assert_eq!(result.status.code(), Some(125));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("OPENTOPIA_SANDBOX_ERROR"));
    assert!(stderr.contains("disabled after elevated setup"));
    assert!(stderr.contains("credential-test"));
    std::fs::remove_dir_all(root).expect("remove credential fixture");
}

#[test]
fn restricted_token_preserves_recursive_host_reads() {
    let root = std::env::temp_dir().join(format!("opentopia-read-test-{}", Uuid::new_v4()));
    let workspace = root.join("workspace");
    let readable = root.join("runtime");
    let nested = readable.join("nested").join("value.txt");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(nested.parent().expect("nested parent")).expect("create read root");
    std::fs::write(&nested, "recursive-read-ok").expect("write read fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .args([
            "run",
            "--cwd",
            workspace.to_str().expect("workspace utf-8"),
            "--read-root",
            workspace.to_str().expect("workspace utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--timeout-ms",
            "5000",
            "--",
            "cmd.exe",
            "/d",
            "/c",
            "type",
            nested.to_str().expect("nested path utf-8"),
        ])
        .output()
        .expect("run recursive read test");

    assert!(
        output.status.success(),
        "recursive read failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("recursive-read-ok"));
    std::fs::remove_dir_all(root).expect("remove read fixture");
}

#[test]
fn helper_timeout_terminates_the_process_tree() {
    let root = std::env::temp_dir().join(format!("opentopia-timeout-test-{}", Uuid::new_v4()));
    let workspace = root.join("workspace");
    let survivor = workspace.join("survivor.txt");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let child_script = format!(
        "Start-Sleep -Seconds 2; Set-Content -LiteralPath '{}' -Value survived",
        survivor.display()
    );
    let parent_script = format!(
        "Start-Process powershell.exe -WindowStyle Hidden -ArgumentList @('-NoProfile','-Command',\"{}\"); Start-Sleep -Seconds 30",
        child_script.replace('"', "`\"")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .args([
            "run",
            "--cwd",
            workspace.to_str().expect("workspace utf-8"),
            "--read-root",
            workspace.to_str().expect("workspace utf-8"),
            "--write-root",
            workspace.to_str().expect("workspace utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--timeout-ms",
            "300",
            "--termination-timeout-ms",
            "2000",
            "--",
            "powershell.exe",
            "-NoProfile",
            "-Command",
            &parent_script,
        ])
        .output()
        .expect("run timeout test");
    assert!(
        !output.status.success(),
        "timed command unexpectedly succeeded"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("stage=wait"),
        "timeout did not report its stage: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert!(
        !survivor.exists(),
        "a descendant survived Job Object termination"
    );
    std::fs::remove_dir_all(root).expect("remove timeout fixture");
}

#[test]
fn real_git_status_runs_headlessly_with_protected_metadata() {
    let root = std::env::temp_dir().join(format!("opentopia-git-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("create git fixture");
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .status()
        .expect("start git init");
    assert!(initialized.success(), "git init failed");
    std::fs::write(root.join("tracked.txt"), "content").expect("write git fixture");

    let git = resolve_git_executable();
    let git_root = git
        .parent()
        .and_then(|parent| parent.parent())
        .unwrap_or_else(|| git.parent().expect("git parent"));
    let output = Command::new(env!("CARGO_BIN_EXE_opentopia-sandbox"))
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "run",
            "--cwd",
            root.to_str().expect("workspace utf-8"),
            "--read-root",
            root.to_str().expect("workspace utf-8"),
            "--runtime-root",
            git_root.to_str().expect("git root utf-8"),
            "--write-root",
            root.to_str().expect("workspace utf-8"),
            "--protect",
            root.join(".git").to_str().expect("git metadata utf-8"),
            "--network",
            "internet",
            "--backend",
            "unelevated",
            "--timeout-ms",
            "10000",
            "--",
            git.to_str().expect("git executable utf-8"),
            "--no-pager",
            "-c",
            "core.hooksPath=NUL",
            "-c",
            "core.fsmonitor=false",
            "status",
            "--porcelain=v2",
            "--branch",
        ])
        .output()
        .expect("run sandboxed git status");
    assert!(
        output.status.success(),
        "sandboxed git status failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).expect("remove git fixture");
}

fn resolve_git_executable() -> std::path::PathBuf {
    let output = Command::new("where.exe")
        .arg("git.exe")
        .output()
        .expect("locate git.exe");
    assert!(output.status.success(), "git.exe is unavailable");
    let first = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .expect("where.exe returned no git path")
        .trim()
        .to_string();
    std::path::PathBuf::from(first)
}
