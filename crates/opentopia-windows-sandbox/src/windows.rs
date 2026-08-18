use super::{BackendMode, NetworkMode, SandboxRequest};
use crate::process_env::current_environment_block;
use anyhow::{Context, Result};
use opentopia_sandbox_protocol::{FilesystemCapabilities, ReadExecuteCapability, ReadProvisioning};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::{
    CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, SET_ACCESS,
    SE_REGISTRY_KEY, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, PSID, SUB_CONTAINERS_ONLY_INHERIT};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_DELETE_CHILD, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    WRITE_DAC,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
};
use windows_sys::Win32::System::ErrorReporting::{WerSetFlags, WER_FAULT_REPORTING_NO_UI};
use windows_sys::Win32::System::JobObjects::TerminateJobObject;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenCurrentUser, KEY_ALL_ACCESS, KEY_READ,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessWithLogonW, GetExitCodeProcess, TerminateProcess,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, STARTUPINFOW,
};
const TRUSTEE_IS_UNKNOWN_VALUE: i32 = TRUSTEE_IS_UNKNOWN;
const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;
const WIN_WORLD_SID: i32 = 1;
const GENERIC_ALL: u32 = 0x1000_0000;
const WORKSPACE_WRITE_PERMISSIONS: u32 =
    FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | FILE_GENERIC_WRITE | DELETE | FILE_DELETE_CHILD;
const WRITE_RESTRICTION_PERMISSIONS: u32 = FILE_GENERIC_WRITE | DELETE | FILE_DELETE_CHILD;
const ACL_ENTRY_PERMISSIONS_VERSION: u32 = 2;

#[repr(C)]
struct TokenDefaultDaclInfo {
    default_dacl: *mut windows_sys::Win32::Security::ACL,
}

mod acl_persistence;
mod acl_transaction;
mod process_launch;
mod security_token;

use acl_persistence::{
    account_sid, acl_principal_sid, capability_principal, ensure_broker_exchange_permissions,
    ensure_persistent_capability_permissions, ensure_persistent_user_permissions,
    migrate_legacy_dedicated_user_acls, path_starts_with, verify_persistent_capability_permissions,
    verify_persistent_user_permissions,
};
pub(super) use acl_persistence::{
    cleanup_workspace_acl, has_dedicated_user_permissions, revoke_dedicated_user_permissions,
};
use acl_transaction::recover_acl_transactions;
use process_launch::{
    argv_to_command_line, create_job, last_error, launch, native_path, wide, AttributeList,
};
use security_token::{effective_file_access, LoggedOnToken, RestrictedToken, SidBuffer};

pub(super) fn run(request: SandboxRequest) -> Result<i32> {
    suppress_process_error_ui();
    crate::logging::event(
        "backend_select",
        format!(
            "requested={:?} network={:?}",
            request.backend, request.network
        ),
    );
    match request.backend {
        BackendMode::DedicatedUser if request.interactive => anyhow::bail!(
            "stage=validate_policy interactive PTY sessions currently require the unelevated Windows sandbox backend"
        ),
        BackendMode::Auto if request.interactive && crate::setup::credentials_present() => anyhow::bail!(
            "stage=validate_policy interactive PTY sessions currently require the unelevated Windows sandbox backend"
        ),
        BackendMode::Unelevated if crate::setup::credentials_present() => anyhow::bail!(
            "stage=validate_policy unelevated execution is disabled while dedicated-user credentials are installed because it shares the host identity; use auto/dedicated-user, or remove the dedicated-user sandbox first"
        ),
        BackendMode::Unelevated if !request.filesystem.deny_read.is_empty() => anyhow::bail!(
            "stage=validate_policy unelevated backend cannot enforce deny-read requirements"
        ),
        BackendMode::Unelevated => run_unelevated(request),
        BackendMode::DedicatedUser => run_dedicated_user(request),
        BackendMode::Auto if crate::setup::credentials_present() => run_dedicated_user(request),
        BackendMode::Auto => run_unelevated(request),
    }
}

/// Persist the ACLs for one policy scope before it is used. This command is
/// deliberately separate from `run`: command startup only verifies policy and
/// never rewrites ACLs on workspace or external runtime paths.
pub(super) fn provision(request: SandboxRequest) -> Result<i32> {
    suppress_process_error_ui();
    validate_managed_runtime_home(&request)?;
    recover_acl_transactions().context("stage=apply_acl recover interrupted ACL transaction")?;
    match request.backend {
        BackendMode::DedicatedUser if request.interactive => anyhow::bail!(
            "stage=validate_policy interactive PTY sessions currently require the unelevated Windows sandbox backend"
        ),
        BackendMode::Auto if request.interactive && crate::setup::credentials_present() => anyhow::bail!(
            "stage=validate_policy interactive PTY sessions currently require the unelevated Windows sandbox backend"
        ),
        BackendMode::Unelevated if crate::setup::credentials_present() => anyhow::bail!(
            "stage=validate_policy unelevated execution is disabled while dedicated-user credentials are installed because it shares the host identity; use auto/dedicated-user, or remove the dedicated-user sandbox first"
        ),
        BackendMode::Unelevated => {
            let principal = capability_principal(&request);
            let mut capability = acl_principal_sid(&principal)?;
            ensure_persistent_capability_permissions(&request, &principal, capability.as_ptr())
                .context("stage=apply_acl provision capability permissions")?;
            Ok(0)
        }
        BackendMode::Auto if !crate::setup::credentials_present() => {
            let principal = capability_principal(&request);
            let mut capability = acl_principal_sid(&principal)?;
            ensure_persistent_capability_permissions(&request, &principal, capability.as_ptr())
                .context("stage=apply_acl provision capability permissions")?;
            Ok(0)
        }
        BackendMode::DedicatedUser | BackendMode::Auto => {
            provision_dedicated_user(&request)?;
            Ok(0)
        }
    }
}

fn validate_managed_runtime_home(request: &SandboxRequest) -> Result<()> {
    let Some(runtime_home) = request.filesystem.runtime_home.as_deref() else {
        if matches!(request.backend, BackendMode::DedicatedUser)
            || matches!(request.backend, BackendMode::Auto) && crate::setup::credentials_present()
        {
            anyhow::bail!(
                "stage=validate_policy dedicated-user launches require a managed runtime home"
            )
        }
        return Ok(());
    };
    let state_dir = crate::setup::state_dir();
    let workspace_managed = request
        .filesystem
        .read_execute
        .iter()
        .filter(|capability| capability.provisioning == ReadProvisioning::Managed)
        .any(|capability| path_starts_with(runtime_home, &capability.path));
    let state_managed = path_starts_with(runtime_home, &state_dir);
    anyhow::ensure!(
        workspace_managed || state_managed,
        "stage=validate_policy runtime home {} is outside OpenTopia-managed workspace/state roots; refusing broad ACL provisioning",
        runtime_home.display()
    );
    anyhow::ensure!(
        runtime_home.parent().is_some() && runtime_home.components().count() > 2,
        "stage=validate_policy runtime home {} is too broad for recursive ACL provisioning",
        runtime_home.display()
    );
    Ok(())
}

pub(super) fn prepare_setup_canaries() -> Result<()> {
    for network in [NetworkMode::Deny, NetworkMode::Internet] {
        let request = setup_canary_request(network)?;
        provision_dedicated_user(&request)
            .with_context(|| format!("provision {:?} dedicated-user execution canary", network))?;
    }
    Ok(())
}

pub(super) fn verify_setup_canaries() -> Vec<String> {
    [
        (NetworkMode::Deny, "offline"),
        (NetworkMode::Internet, "online"),
    ]
    .into_iter()
    .filter_map(|(network, label)| {
        let executable = std::env::current_exe();
        let output = executable.and_then(|executable| {
            std::process::Command::new(executable)
                .args([
                    "canary",
                    "--network",
                    match network {
                        NetworkMode::Deny => "deny",
                        NetworkMode::Internet => "internet",
                    },
                ])
                .output()
        });
        match output {
            Ok(output) if output.status.success() => None,
            Ok(output) => Some(format!(
                "{label} sandbox execution canary failed (exit {}): {}",
                output.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            Err(error) => Some(format!("{label} sandbox execution canary failed: {error}")),
        }
    })
    .collect()
}

pub(super) fn run_setup_canary(args: &[String]) -> Result<i32> {
    let network = match args {
        [flag, value] if flag == "--network" && value == "deny" => NetworkMode::Deny,
        [flag, value] if flag == "--network" && value == "internet" => NetworkMode::Internet,
        _ => anyhow::bail!("usage: opentopia-sandbox canary --network <deny|internet>"),
    };
    run_dedicated_user(setup_canary_request(network)?)
}

fn setup_canary_request(network: NetworkMode) -> Result<SandboxRequest> {
    let root = crate::setup::state_dir().join("canary");
    let workspace_path = root.join("workspace");
    let home_path = root.join(match network {
        NetworkMode::Deny => "offline-home",
        NetworkMode::Internet => "online-home",
    });
    std::fs::create_dir_all(&workspace_path).with_context(|| {
        format!(
            "create sandbox canary workspace {}",
            workspace_path.display()
        )
    })?;
    std::fs::create_dir_all(&home_path)
        .with_context(|| format!("create sandbox canary home {}", home_path.display()))?;
    let workspace = workspace_path
        .canonicalize()
        .context("canonicalize sandbox canary workspace")?;
    let home = home_path
        .canonicalize()
        .context("canonicalize sandbox canary home")?;
    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .context("SystemRoot is unavailable for sandbox canary")?
        .canonicalize()
        .context("canonicalize SystemRoot for sandbox canary")?;
    let command = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    anyhow::ensure!(
        command.is_file(),
        "PowerShell execution canary is unavailable at {}",
        command.display()
    );
    Ok(SandboxRequest {
        interactive: false,
        cwd: workspace.clone(),
        filesystem: FilesystemCapabilities {
            read_execute: vec![
                ReadExecuteCapability {
                    path: workspace.clone(),
                    provisioning: ReadProvisioning::Managed,
                },
                ReadExecuteCapability {
                    path: system_root,
                    provisioning: ReadProvisioning::ExistingOnly,
                },
            ],
            write: vec![workspace, home.clone()],
            runtime_home: Some(home),
            ..Default::default()
        },
        network,
        timeout_ms: Some(10_000),
        termination_timeout_ms: 5_000,
        max_memory_bytes: None,
        max_cpu_time_ms: None,
        max_output_bytes: Some(64 * 1024),
        backend: BackendMode::DedicatedUser,
        command: vec![
            native_path(&command).to_string_lossy().into_owned(),
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "Write-Output opentopia-sandbox-canary".to_string(),
        ],
    })
}

fn provision_dedicated_user(request: &SandboxRequest) -> Result<()> {
    let credentials = crate::setup::load_credentials().map_err(|error| {
        anyhow::anyhow!("stage=prepare_sandbox dedicated-user backend unavailable: {error:#}")
    })?;
    let (username, password) = match request.network {
        NetworkMode::Deny => (
            credentials.offline_username.as_str(),
            credentials.offline_password.as_str(),
        ),
        NetworkMode::Internet => (
            credentials.online_username.as_str(),
            credentials.online_password.as_str(),
        ),
    };
    let mut user_sid = account_sid(username)?;
    let logon_token = LoggedOnToken::new(username, password)
        .context("stage=prepare_sandbox log on dedicated-user identity")?;
    provision_runtime_registry_as_user(username, password)?;
    ensure_broker_exchange_permissions(username, user_sid.as_ptr(), logon_token.handle)?;
    migrate_legacy_dedicated_user_acls(request, username)?;
    ensure_persistent_user_permissions(request, username, user_sid.as_ptr(), logon_token.handle)?;
    let capability_principal = capability_principal(request);
    let mut capability = acl_principal_sid(&capability_principal)?;
    ensure_persistent_capability_permissions(request, &capability_principal, capability.as_ptr())
        .context("stage=apply_acl provision dedicated-user capability permissions")
}

fn broker_exchange_root(account: &str) -> std::path::PathBuf {
    crate::setup::state_dir()
        .join("broker-exchange")
        .join(account.to_ascii_lowercase())
}

pub(super) fn provision_runtime_registry(args: &[String]) -> Result<i32> {
    anyhow::ensure!(
        args.is_empty(),
        "usage: opentopia-sandbox registry-provision"
    );
    let mut runtime_sid = SidBuffer::opentopia_runtime();
    ensure_runtime_registry_access(runtime_sid.as_ptr())
        .context("stage=apply_acl provision dedicated-user runtime registry access")?;
    Ok(0)
}

fn provision_runtime_registry_as_user(username: &str, password: &str) -> Result<()> {
    let executable =
        std::env::current_exe().context("stage=apply_acl resolve runtime registry provisioner")?;
    let command = vec![
        native_path(&executable).to_string_lossy().into_owned(),
        "registry-provision".to_string(),
    ];
    let mut command_line = wide(argv_to_command_line(&command));
    let executable_w = wide(native_path(&executable).as_os_str());
    let username_w = wide(OsStr::new(username));
    let domain_w = wide(OsStr::new("."));
    let password_w = wide(OsStr::new(password));
    let system_root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .context("SystemRoot is unavailable for runtime registry preparation")?;
    let cwd_w = wide(system_root.as_os_str());
    let environment = current_environment_block(None, None, true);
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessWithLogonW(
            username_w.as_ptr(),
            domain_w.as_ptr(),
            password_w.as_ptr(),
            LOGON_WITH_PROFILE,
            executable_w.as_ptr(),
            command_line.as_mut_ptr(),
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd_w.as_ptr(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(last_error(
            "stage=apply_acl CreateProcessWithLogonW runtime registry provisioner",
        ));
    }
    unsafe { CloseHandle(process.hThread) };
    let waited = unsafe { WaitForSingleObject(process.hProcess, 30_000) };
    if waited == WAIT_TIMEOUT {
        unsafe {
            TerminateProcess(process.hProcess, crate::SANDBOX_ERROR_EXIT_CODE as u32);
            CloseHandle(process.hProcess);
        }
        anyhow::bail!("stage=apply_acl runtime registry provisioner timed out after 30000ms")
    }
    if waited == u32::MAX {
        unsafe { CloseHandle(process.hProcess) };
        return Err(last_error(
            "stage=apply_acl wait for runtime registry provisioner",
        ));
    }
    let mut exit_code = 1_u32;
    let read = unsafe { GetExitCodeProcess(process.hProcess, &mut exit_code) };
    unsafe { CloseHandle(process.hProcess) };
    if read == 0 {
        return Err(last_error(
            "stage=apply_acl read runtime registry provisioner exit code",
        ));
    }
    anyhow::ensure!(
        exit_code == 0,
        "stage=apply_acl runtime registry provisioner exited with code {exit_code}"
    );
    Ok(())
}

fn run_unelevated(request: SandboxRequest) -> Result<i32> {
    crate::logging::event(
        "prepare_sandbox",
        "starting unelevated WRITE_RESTRICTED backend",
    );
    if request.network == NetworkMode::Deny {
        anyhow::bail!(
            "stage=validate_policy unelevated backend cannot authoritatively enforce offline networking; configure the dedicated-user backend or allow network"
        )
    }
    let capability_principal = capability_principal(&request);
    let mut capability = acl_principal_sid(&capability_principal)?;
    let capability_sid = capability.as_ptr();
    verify_persistent_capability_permissions(&request, &capability_principal, capability_sid)
        .context("stage=verify_acl verify capability permissions")?;

    let restricted_token = RestrictedToken::for_capability(capability_sid)
        .context("stage=prepare_sandbox create restricted token")?;
    crate::logging::event(
        "spawn",
        format!(
            "backend=unelevated program={} write_roots={} userprofile={} home={} temp={}",
            request
                .command
                .first()
                .map(String::as_str)
                .unwrap_or("<missing>"),
            request.filesystem.write.len(),
            std::env::var("USERPROFILE").unwrap_or_default(),
            std::env::var("HOME").unwrap_or_default(),
            std::env::var("TEMP").unwrap_or_default(),
        ),
    );
    let exit_code = launch(&request, restricted_token.handle)
        .context("stage=spawn launch unelevated target")?;
    Ok(exit_code as i32)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DedicatedUserRunnerResult {
    exit_code: i32,
    error: Option<String>,
}

const DEDICATED_USER_RUNNER_PROTOCOL_VERSION: u32 = 2;
const LEGACY_CAPABILITY_PRINCIPAL: &str = "opentopia:unelevated-capability:v1";
const LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX: &str = "opentopia:unelevated-capability:v2:";
const CAPABILITY_PRINCIPAL_PREFIX: &str = "opentopia:filesystem-capability:v3:";
const CAPABILITY_NAMESPACE: u128 = 0xa678_2ac1_8754_5ef2_99b0_b62a_15c7_c90e;

#[derive(serde::Serialize, serde::Deserialize)]
struct DedicatedUserRunnerRequestEnvelope {
    protocol_version: u32,
    request: SandboxRequest,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DedicatedUserRunnerResultEnvelope {
    protocol_version: u32,
    result: DedicatedUserRunnerResult,
}

fn run_dedicated_user(request: SandboxRequest) -> Result<i32> {
    validate_managed_runtime_home(&request)?;
    crate::logging::event("prepare_sandbox", "starting dedicated-user backend");
    let credentials = crate::setup::load_credentials().map_err(|error| {
        anyhow::anyhow!("stage=prepare_sandbox dedicated-user backend unavailable: {error:#}")
    })?;
    crate::logging::event("prepare_sandbox", "loaded dedicated-user credentials");
    let (username, password) = match request.network {
        NetworkMode::Deny => (
            credentials.offline_username.as_str(),
            credentials.offline_password.as_str(),
        ),
        NetworkMode::Internet => (
            credentials.online_username.as_str(),
            credentials.online_password.as_str(),
        ),
    };
    let _user_sid = account_sid(username)?;
    crate::logging::event(
        "prepare_sandbox",
        format!("resolved dedicated-user SID for {username}"),
    );
    let logon_token = LoggedOnToken::new(username, password)
        .context("stage=prepare_sandbox log on dedicated-user identity")?;
    verify_persistent_user_permissions(&request, username, logon_token.handle)?;
    let capability_principal = capability_principal(&request);
    let mut capability = acl_principal_sid(&capability_principal)?;
    let capability_sid = capability.as_ptr();
    verify_persistent_capability_permissions(&request, &capability_principal, capability_sid)
        .context("stage=verify_acl verify dedicated-user capability permissions")?;

    let exchange_root = broker_exchange_root(username);
    anyhow::ensure!(
        exchange_root.is_dir(),
        "stage=provision_acl broker exchange root is not prepared for {username}: {}",
        exchange_root.display()
    );
    anyhow::ensure!(
        effective_file_access(
            logon_token.handle,
            &exchange_root,
            WORKSPACE_WRITE_PERMISSIONS
        )?,
        "stage=provision_acl broker exchange root is not prepared for {username}: {}",
        exchange_root.display()
    );
    let run_root = exchange_root.join(Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&run_root).with_context(|| {
        format!(
            "stage=prepare_sandbox create dedicated-user run directory {}",
            run_root.display()
        )
    })?;
    let _cleanup = RunDirectory(run_root.clone());

    let request_path = run_root.join("request.json");
    let result_path = run_root.join("result.json");
    let stdout_path = run_root.join("stdout.bin");
    let stderr_path = run_root.join("stderr.bin");
    crate::setup::ensure_parent(&request_path)?;
    std::fs::write(
        &request_path,
        serde_json::to_vec(&DedicatedUserRunnerRequestEnvelope {
            protocol_version: DEDICATED_USER_RUNNER_PROTOCOL_VERSION,
            request: request.clone(),
        })?,
    )?;

    let executable = std::env::current_exe().context("stage=spawn resolve sandbox runner")?;
    let command = vec![
        executable.to_string_lossy().into_owned(),
        "runner".to_string(),
        "--request".to_string(),
        request_path.to_string_lossy().into_owned(),
        "--result".to_string(),
        result_path.to_string_lossy().into_owned(),
        "--stdout".to_string(),
        stdout_path.to_string_lossy().into_owned(),
        "--stderr".to_string(),
        stderr_path.to_string_lossy().into_owned(),
    ];
    let mut command_line = wide(argv_to_command_line(&command));
    let executable_w = wide(executable.as_os_str());
    let username_w = wide(OsStr::new(username));
    let domain_w = wide(OsStr::new("."));
    let password_w = wide(OsStr::new(password));
    let cwd_w = wide(native_path(&request.cwd).as_os_str());
    let environment = current_environment_block(Some(&request.cwd), None, true);
    crate::logging::event(
        "spawn",
        format!("starting dedicated-user runner user={username}"),
    );
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessWithLogonW(
            username_w.as_ptr(),
            domain_w.as_ptr(),
            password_w.as_ptr(),
            LOGON_WITH_PROFILE,
            executable_w.as_ptr(),
            command_line.as_mut_ptr(),
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            cwd_w.as_ptr(),
            &startup,
            &mut process,
        )
    };
    if created == 0 {
        return Err(last_error("stage=spawn CreateProcessWithLogonW"));
    }
    crate::logging::event("spawn", format!("dedicated-user runner user={username}"));
    unsafe { CloseHandle(process.hThread) };
    let broker_timeout = request
        .timeout_ms
        .unwrap_or(30_000)
        .saturating_add(request.termination_timeout_ms)
        .saturating_add(15_000)
        .min(u32::MAX as u64) as u32;
    let waited = unsafe { WaitForSingleObject(process.hProcess, broker_timeout) };
    if waited == WAIT_TIMEOUT {
        if let Ok(trace) = std::fs::read_to_string(run_root.join("runner.log")) {
            crate::logging::event("runner_trace", trace);
        }
        unsafe {
            TerminateProcess(process.hProcess, 124);
            WaitForSingleObject(
                process.hProcess,
                request.termination_timeout_ms.min(u32::MAX as u64) as u32,
            );
            CloseHandle(process.hProcess);
        }
        anyhow::bail!(
            "stage=wait dedicated-user runner exceeded the command lifecycle timeout of {broker_timeout}ms"
        )
    }
    if waited == u32::MAX {
        unsafe { CloseHandle(process.hProcess) };
        return Err(last_error("stage=wait dedicated-user runner"));
    }
    unsafe { CloseHandle(process.hProcess) };

    forward_file(&stdout_path, std::io::stdout())?;
    forward_file(&stderr_path, std::io::stderr())?;
    let runner_result: DedicatedUserRunnerResultEnvelope = serde_json::from_slice(
        &std::fs::read(&result_path).context("stage=collect_output read runner result")?,
    )
    .context("stage=collect_output parse runner result")?;
    if runner_result.protocol_version != DEDICATED_USER_RUNNER_PROTOCOL_VERSION {
        anyhow::bail!(
            "stage=collect_output dedicated-user runner protocol mismatch: expected {} got {}",
            DEDICATED_USER_RUNNER_PROTOCOL_VERSION,
            runner_result.protocol_version
        )
    }
    if let Some(error) = runner_result.result.error {
        anyhow::bail!("{error}")
    }
    Ok(runner_result.result.exit_code)
}

pub(super) fn run_dedicated_user_runner(args: &[String]) -> Result<i32> {
    suppress_process_error_ui();
    let mut request_path = None;
    let mut result_path = None;
    let mut stdout_path = None;
    let mut stderr_path = None;
    let mut index = 0;
    while index < args.len() {
        let target = match args[index].as_str() {
            "--request" => &mut request_path,
            "--result" => &mut result_path,
            "--stdout" => &mut stdout_path,
            "--stderr" => &mut stderr_path,
            value => anyhow::bail!("unexpected dedicated-user runner argument: {value}"),
        };
        index += 1;
        let value = args
            .get(index)
            .with_context(|| format!("missing value for {}", args[index - 1]))?;
        *target = Some(std::path::PathBuf::from(value));
        index += 1;
    }
    let request_path = request_path.context("missing --request")?;
    let result_path = result_path.context("missing --result")?;
    let stdout_path = stdout_path.context("missing --stdout")?;
    let stderr_path = stderr_path.context("missing --stderr")?;
    let runner_log_path = request_path
        .parent()
        .map(|parent| parent.join("runner.log"));
    runner_trace(&runner_log_path, "dedicated-user runner started");
    let envelope: DedicatedUserRunnerRequestEnvelope =
        serde_json::from_slice(&std::fs::read(&request_path)?)?;
    if envelope.protocol_version != DEDICATED_USER_RUNNER_PROTOCOL_VERSION {
        anyhow::bail!(
            "stage=prepare_sandbox dedicated-user runner protocol mismatch: expected {} got {}",
            DEDICATED_USER_RUNNER_PROTOCOL_VERSION,
            envelope.protocol_version
        )
    }
    runner_trace(
        &runner_log_path,
        &format!(
            "launching target program={}",
            envelope
                .request
                .command
                .first()
                .map(String::as_str)
                .unwrap_or("<missing>")
        ),
    );
    let result = match launch_dedicated_user_target(&envelope.request, &stdout_path, &stderr_path) {
        Ok(exit_code) => DedicatedUserRunnerResult {
            exit_code,
            error: None,
        },
        Err(error) => DedicatedUserRunnerResult {
            exit_code: 1,
            error: Some(format!("{error:#}")),
        },
    };
    std::fs::write(
        result_path,
        serde_json::to_vec(&DedicatedUserRunnerResultEnvelope {
            protocol_version: DEDICATED_USER_RUNNER_PROTOCOL_VERSION,
            result,
        })?,
    )?;
    Ok(0)
}

fn runner_trace(path: &Option<std::path::PathBuf>, message: &str) {
    if let Some(path) = path {
        let _ = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| writeln!(file, "{message}"));
    }
}

fn suppress_process_error_ui() {
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX | SEM_NOOPENFILEERRORBOX);
        let _ = WerSetFlags(WER_FAULT_REPORTING_NO_UI);
    }
}

/// Grant a stable runtime-only SID access to the dedicated account's own
/// registry hive. That SID is intentionally never written to a host filesystem
/// ACL, so it can support per-user initialization without widening the
/// per-launch filesystem capability or coupling concurrent launch sessions.
fn ensure_runtime_registry_access(runtime_sid: PSID) -> Result<()> {
    let mut key = ptr::null_mut();
    let opened = unsafe { RegOpenCurrentUser(KEY_READ | WRITE_DAC, &mut key) };
    if opened != 0 || key.is_null() {
        anyhow::bail!("RegOpenCurrentUser for logon SID failed: {opened}")
    }

    let mut old_dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let get_result = unsafe {
        GetSecurityInfo(
            key.cast(),
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if get_result != 0 {
        unsafe { RegCloseKey(key) };
        anyhow::bail!("GetSecurityInfo(HKCU) failed: {get_result}")
    }

    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: KEY_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: SUB_CONTAINERS_ONLY_INHERIT,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN_VALUE,
            ptstrName: runtime_sid.cast(),
        },
    };
    let mut new_dacl = ptr::null_mut();
    let entries_result = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl) };
    if !descriptor.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(descriptor.cast()) };
    }
    if entries_result != 0 {
        unsafe { RegCloseKey(key) };
        anyhow::bail!("SetEntriesInAclW(HKCU) failed: {entries_result}")
    }
    let set_result = unsafe {
        SetSecurityInfo(
            key.cast(),
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            new_dacl,
            ptr::null_mut(),
        )
    };
    if !new_dacl.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(new_dacl.cast()) };
    }
    unsafe { RegCloseKey(key) };
    if set_result != 0 {
        anyhow::bail!("SetSecurityInfo(HKCU) failed: {set_result}")
    }
    Ok(())
}

fn launch_dedicated_user_target(
    request: &SandboxRequest,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<i32> {
    let capability_principal = capability_principal(request);
    let mut capability = acl_principal_sid(&capability_principal)?;
    let restricted_token = RestrictedToken::for_dedicated_capability(capability.as_ptr())
        .context("stage=prepare_sandbox create dedicated-user restricted token")?;
    crate::logging::event("runner", "created dedicated-user restricted token");
    let stdin = File::open("NUL").context("stage=spawn open NUL stdin")?;
    let stdout = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(stdout_path)
        .context("stage=spawn create stdout capture")?;
    let stderr = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(stderr_path)
        .context("stage=spawn create stderr capture")?;
    let handles = [
        stdin.as_raw_handle() as HANDLE,
        stdout.as_raw_handle() as HANDLE,
        stderr.as_raw_handle() as HANDLE,
    ];
    for handle in handles {
        let inherited =
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        if inherited == 0 {
            return Err(last_error("stage=spawn SetHandleInformation"));
        }
    }

    let job = create_job(request)?;
    let mut attributes = AttributeList::new(2)?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        (&job as *const HANDLE).cast(),
        std::mem::size_of::<HANDLE>(),
    )?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        handles.as_ptr().cast(),
        std::mem::size_of_val(&handles),
    )?;
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    startup.lpAttributeList = attributes.as_mut_ptr();
    let mut command_line = wide(argv_to_command_line(&request.command));
    let cwd = wide(native_path(&request.cwd).as_os_str());
    let profile_home =
        request.filesystem.runtime_home.clone().context(
            "stage=validate_policy dedicated-user launches require a managed runtime home",
        )?;
    let environment = current_environment_block(Some(&request.cwd), Some(&profile_home), false);
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessAsUserW(
            restricted_token.handle,
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process,
        )
    };
    if created == 0 {
        unsafe { CloseHandle(job) };
        return Err(last_error("stage=spawn CreateProcessAsUserW"));
    }
    crate::logging::event(
        "runner",
        format!("target created pid={}", process.dwProcessId),
    );
    unsafe { CloseHandle(process.hThread) };
    let started = std::time::Instant::now();
    let stop_reason = loop {
        if let Some(limit) = request.max_output_bytes {
            let captured = captured_output_bytes(stdout_path, stderr_path)?;
            if captured > limit {
                break Some((
                    123,
                    format!(
                        "stage=collect_output command exceeded the combined output limit of {limit} bytes"
                    ),
                ));
            }
        }
        let wait_slice = match request.timeout_ms {
            Some(timeout) => {
                let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                if elapsed >= timeout {
                    break Some((
                        124,
                        format!(
                            "stage=wait command timed out after {timeout}ms; dedicated-user process tree terminated"
                        ),
                    ));
                }
                timeout.saturating_sub(elapsed).min(10) as u32
            }
            None => 10,
        };
        let waited = unsafe { WaitForSingleObject(process.hProcess, wait_slice) };
        if waited == 0 {
            break None;
        }
        if waited == u32::MAX {
            unsafe {
                CloseHandle(process.hProcess);
                CloseHandle(job);
            }
            return Err(last_error("stage=wait WaitForSingleObject"));
        }
    };
    if let Some((exit_code, reason)) = stop_reason {
        unsafe { TerminateJobObject(job, exit_code) };
        let stopped = unsafe {
            WaitForSingleObject(
                process.hProcess,
                request.termination_timeout_ms.min(u32::MAX as u64) as u32,
            )
        };
        unsafe {
            CloseHandle(process.hProcess);
            CloseHandle(job);
        }
        if stopped == WAIT_TIMEOUT {
            anyhow::bail!(
                "stage=terminate dedicated-user process tree did not exit within {}ms after {reason}",
                request.termination_timeout_ms
            )
        }
        anyhow::bail!(reason)
    }
    if let Some(limit) = request.max_output_bytes {
        let captured = captured_output_bytes(stdout_path, stderr_path)?;
        if captured > limit {
            unsafe {
                CloseHandle(process.hProcess);
                CloseHandle(job);
            }
            anyhow::bail!(
                "stage=collect_output command exceeded the combined output limit of {limit} bytes"
            )
        }
    }
    let mut exit_code = 1;
    let got_code = unsafe { GetExitCodeProcess(process.hProcess, &mut exit_code) };
    unsafe {
        CloseHandle(process.hProcess);
        CloseHandle(job);
    }
    if got_code == 0 {
        return Err(last_error("stage=wait GetExitCodeProcess"));
    }
    Ok(exit_code as i32)
}

fn captured_output_bytes(stdout_path: &Path, stderr_path: &Path) -> Result<u64> {
    let stdout = std::fs::metadata(stdout_path)
        .context("stage=collect_output inspect stdout capture")?
        .len();
    let stderr = std::fs::metadata(stderr_path)
        .context("stage=collect_output inspect stderr capture")?
        .len();
    Ok(stdout.saturating_add(stderr))
}

fn normalized_capability_path(path: &Path) -> String {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    normalized
        .strip_prefix("\\\\?\\")
        .unwrap_or(&normalized)
        .to_string()
}

fn forward_file(path: &Path, mut writer: impl Write) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        writer.write_all(&buffer[..count])?;
    }
    writer.flush()?;
    Ok(())
}

struct RunDirectory(std::path::PathBuf);

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BackendMode, DedicatedUserRunnerRequestEnvelope, NetworkMode, SandboxRequest,
        DEDICATED_USER_RUNNER_PROTOCOL_VERSION,
    };
    use opentopia_sandbox_protocol::ReadProvisioning;
    use std::path::Path;

    #[test]
    fn dedicated_user_runner_envelopes_are_explicitly_versioned() {
        let request = SandboxRequest {
            interactive: false,
            cwd: Path::new(r"C:\workspace").to_path_buf(),
            filesystem: Default::default(),
            network: NetworkMode::Deny,
            timeout_ms: Some(1_000),
            termination_timeout_ms: 500,
            max_memory_bytes: None,
            max_cpu_time_ms: None,
            max_output_bytes: None,
            backend: BackendMode::DedicatedUser,
            command: vec!["cmd.exe".to_string()],
        };
        let encoded = serde_json::to_vec(&DedicatedUserRunnerRequestEnvelope {
            protocol_version: DEDICATED_USER_RUNNER_PROTOCOL_VERSION,
            request,
        })
        .expect("serialize request envelope");
        let decoded: DedicatedUserRunnerRequestEnvelope =
            serde_json::from_slice(&encoded).expect("deserialize request envelope");
        assert_eq!(
            decoded.protocol_version,
            DEDICATED_USER_RUNNER_PROTOCOL_VERSION
        );
    }

    #[test]
    fn runtime_capabilities_default_to_external_existing_access() {
        let capability = opentopia_sandbox_protocol::ReadExecuteCapability {
            path: Path::new(r"J:\Python311").to_path_buf(),
            provisioning: ReadProvisioning::ExistingOnly,
        };
        assert_eq!(capability.provisioning, ReadProvisioning::ExistingOnly);
    }

    #[test]
    fn run_path_only_verifies_persistent_acl_state() {
        let source = include_str!("windows.rs");
        let dedicated = source
            .split("fn run_dedicated_user(request")
            .nth(1)
            .expect("dedicated run")
            .split("pub(super) fn run_dedicated_user_runner")
            .next()
            .expect("dedicated run boundary");
        assert!(dedicated.contains("verify_persistent_user_permissions"));
        assert!(dedicated.contains("verify_persistent_capability_permissions"));
        assert!(!dedicated.contains("ensure_persistent_user_permissions"));
        assert!(!dedicated.contains("ensure_persistent_capability_permissions"));
    }
}
