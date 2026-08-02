#![cfg(windows)]

use std::net::TcpListener;
use std::process::Command;
use uuid::Uuid;

#[test]
fn appcontainer_can_write_only_to_granted_workspace() {
    let root = std::env::temp_dir().join(format!("opentopia-sandbox-test-{}", Uuid::new_v4()));
    let workspace = root.join("workspace");
    let inside = workspace.join("inside.txt");
    let outside = root.join("outside.txt");
    let protected_directory = workspace.join(".git");
    let protected = protected_directory.join("config");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::create_dir_all(&protected_directory).expect("create protected directory");

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
            "deny",
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
            "deny",
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
            "deny",
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
        !protected.exists(),
        "sandbox must preserve protected metadata"
    );
    std::fs::remove_dir_all(root).expect("remove sandbox fixture");
}

#[test]
fn appcontainer_denies_loopback_network_without_an_internet_capability() {
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
        "network-denied sandbox reached a loopback listener: stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    drop(listener);
    std::fs::remove_dir_all(root).expect("remove network fixture");
}
