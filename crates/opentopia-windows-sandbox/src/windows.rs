use super::BackendMode;
use super::NetworkMode;
use super::SandboxRequest;
use crate::process_env::current_environment_block;
use anyhow::Context;
use anyhow::Result;
use opentopia_sandbox_protocol::{FilesystemCapabilities, ReadExecuteCapability, ReadProvisioning};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Foundation::SetHandleInformation;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::HANDLE_FLAG_INHERIT;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
use windows_sys::Win32::Security::AccessCheck;
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::GetSecurityInfo;
use windows_sys::Win32::Security::Authorization::SetEntriesInAclW;
use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
use windows_sys::Win32::Security::Authorization::DENY_ACCESS;
use windows_sys::Win32::Security::Authorization::EXPLICIT_ACCESS_W;
use windows_sys::Win32::Security::Authorization::GRANT_ACCESS;
use windows_sys::Win32::Security::Authorization::REVOKE_ACCESS;
use windows_sys::Win32::Security::Authorization::SET_ACCESS;
use windows_sys::Win32::Security::Authorization::SE_FILE_OBJECT;
use windows_sys::Win32::Security::Authorization::SE_REGISTRY_KEY;
use windows_sys::Win32::Security::Authorization::TRUSTEE_IS_SID;
use windows_sys::Win32::Security::Authorization::TRUSTEE_IS_UNKNOWN;
use windows_sys::Win32::Security::Authorization::TRUSTEE_W;
use windows_sys::Win32::Security::CopySid;
use windows_sys::Win32::Security::CreateRestrictedToken;
use windows_sys::Win32::Security::CreateWellKnownSid;
use windows_sys::Win32::Security::DuplicateToken;
use windows_sys::Win32::Security::GetLengthSid;
use windows_sys::Win32::Security::GetTokenInformation;
use windows_sys::Win32::Security::LogonUserW;
use windows_sys::Win32::Security::LookupAccountNameW;
use windows_sys::Win32::Security::MapGenericMask;
use windows_sys::Win32::Security::SecurityImpersonation;
use windows_sys::Win32::Security::SetTokenInformation;
use windows_sys::Win32::Security::TokenDefaultDacl;
use windows_sys::Win32::Security::TokenGroups;
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::Security::DISABLE_MAX_PRIVILEGE;
use windows_sys::Win32::Security::GENERIC_MAPPING;
use windows_sys::Win32::Security::GROUP_SECURITY_INFORMATION;
use windows_sys::Win32::Security::LOGON32_LOGON_INTERACTIVE;
use windows_sys::Win32::Security::LOGON32_PROVIDER_DEFAULT;
use windows_sys::Win32::Security::LUA_TOKEN;
use windows_sys::Win32::Security::OWNER_SECURITY_INFORMATION;
use windows_sys::Win32::Security::PRIVILEGE_SET;
use windows_sys::Win32::Security::PSID;
use windows_sys::Win32::Security::SID_AND_ATTRIBUTES;
use windows_sys::Win32::Security::SUB_CONTAINERS_AND_OBJECTS_INHERIT;
use windows_sys::Win32::Security::SUB_CONTAINERS_ONLY_INHERIT;
use windows_sys::Win32::Security::TOKEN_ADJUST_DEFAULT;
use windows_sys::Win32::Security::TOKEN_ADJUST_PRIVILEGES;
use windows_sys::Win32::Security::TOKEN_ADJUST_SESSIONID;
use windows_sys::Win32::Security::TOKEN_ASSIGN_PRIMARY;
use windows_sys::Win32::Security::TOKEN_DUPLICATE;
use windows_sys::Win32::Security::TOKEN_QUERY;
use windows_sys::Win32::Security::WRITE_RESTRICTED;
use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
use windows_sys::Win32::Storage::FileSystem::DELETE;
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows_sys::Win32::Storage::FileSystem::FILE_DELETE_CHILD;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_EXECUTE;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_WRITE;
use windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING;
use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
use windows_sys::Win32::Storage::FileSystem::WRITE_DAC;
use windows_sys::Win32::System::Console::GetStdHandle;
use windows_sys::Win32::System::Console::STD_ERROR_HANDLE;
use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;
use windows_sys::Win32::System::Console::STD_OUTPUT_HANDLE;
use windows_sys::Win32::System::Diagnostics::Debug::SetErrorMode;
use windows_sys::Win32::System::Diagnostics::Debug::SEM_FAILCRITICALERRORS;
use windows_sys::Win32::System::Diagnostics::Debug::SEM_NOGPFAULTERRORBOX;
use windows_sys::Win32::System::Diagnostics::Debug::SEM_NOOPENFILEERRORBOX;
use windows_sys::Win32::System::ErrorReporting::WerSetFlags;
use windows_sys::Win32::System::ErrorReporting::WER_FAULT_REPORTING_NO_UI;
use windows_sys::Win32::System::JobObjects::CreateJobObjectW;
use windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation;
use windows_sys::Win32::System::JobObjects::SetInformationJobObject;
use windows_sys::Win32::System::JobObjects::TerminateJobObject;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_JOB_MEMORY;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_JOB_TIME;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
use windows_sys::Win32::System::Registry::RegCloseKey;
use windows_sys::Win32::System::Registry::RegOpenCurrentUser;
use windows_sys::Win32::System::Registry::KEY_ALL_ACCESS;
use windows_sys::Win32::System::Registry::KEY_READ;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::System::Threading::CreateProcessAsUserW;
use windows_sys::Win32::System::Threading::CreateProcessWithLogonW;
use windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList;
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::Threading::GetExitCodeProcess;
use windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList;
use windows_sys::Win32::System::Threading::OpenProcessToken;
use windows_sys::Win32::System::Threading::ReleaseMutex;
use windows_sys::Win32::System::Threading::TerminateProcess;
use windows_sys::Win32::System::Threading::UpdateProcThreadAttribute;
use windows_sys::Win32::System::Threading::WaitForSingleObject;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT;
use windows_sys::Win32::System::Threading::EXTENDED_STARTUPINFO_PRESENT;
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::System::Threading::LOGON_WITH_PROFILE;
use windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST;
use windows_sys::Win32::System::Threading::PROCESS_INFORMATION;
use windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_HANDLE_LIST;
use windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_JOB_LIST;
use windows_sys::Win32::System::Threading::STARTF_USESTDHANDLES;
use windows_sys::Win32::System::Threading::STARTUPINFOEXW;
use windows_sys::Win32::System::Threading::STARTUPINFOW;
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

fn ensure_broker_exchange_permissions(account: &str, sid: PSID, token: HANDLE) -> Result<()> {
    let path = broker_exchange_root(account);
    std::fs::create_dir_all(&path)
        .with_context(|| format!("create broker exchange root {}", path.display()))?;
    if effective_file_access(token, &path, WORKSPACE_WRITE_PERMISSIONS)? {
        return Ok(());
    }
    let _guards = NamedAclMutex::acquire_paths([path.as_path()])?;
    let mut transaction = AclTransaction::default();
    transaction.grant(&path, sid, true, WORKSPACE_WRITE_PERMISSIONS)?;
    let entry = PersistentAclEntry {
        account: account.to_string(),
        path,
        kind: PersistentAclKind::Write,
        sid: SidBuffer::copy_from_sid(sid)?.0,
        permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
    };
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut ledger = load_acl_ledger()?;
    ledger.entries.retain(|existing| {
        existing.account != entry.account
            || existing.path != entry.path
            || existing.kind != entry.kind
    });
    ledger.entries.push(entry);
    save_acl_ledger(&ledger)?;
    transaction.commit();
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

struct LoggedOnToken {
    handle: HANDLE,
}

impl LoggedOnToken {
    fn new(username: &str, password: &str) -> Result<Self> {
        let username = wide(OsStr::new(username));
        let domain = wide(OsStr::new("."));
        let password = wide(OsStr::new(password));
        let mut handle = ptr::null_mut();
        let logged_on = unsafe {
            LogonUserW(
                username.as_ptr(),
                domain.as_ptr(),
                password.as_ptr(),
                LOGON32_LOGON_INTERACTIVE,
                LOGON32_PROVIDER_DEFAULT,
                &mut handle,
            )
        };
        if logged_on == 0 || handle.is_null() {
            return Err(last_error("LogonUserW"));
        }
        Ok(Self { handle })
    }
}

impl Drop for LoggedOnToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// Evaluate a filesystem DACL using the complete dedicated-user token. This is
/// intentionally an authorization check rather than a path-location heuristic:
/// group membership, explicit denies, ownership, and inherited ACEs all
/// participate in the result exactly as they will for the launched process.
fn effective_file_access(token: HANDLE, path: &Path, desired_access: u32) -> Result<bool> {
    let name = wide(path.as_os_str());
    let mut owner = ptr::null_mut();
    let mut group = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let security_result = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            &mut group,
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if security_result != 0 {
        anyhow::bail!(
            "stage=access_check GetNamedSecurityInfoW failed for {}: {security_result}",
            path.display()
        )
    }

    let mut impersonation_token = ptr::null_mut();
    let duplicated =
        unsafe { DuplicateToken(token, SecurityImpersonation, &mut impersonation_token) };
    if duplicated == 0 || impersonation_token.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(descriptor.cast()) };
        return Err(last_error("stage=access_check DuplicateToken"));
    }

    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ,
        GenericWrite: FILE_GENERIC_WRITE,
        GenericExecute: FILE_GENERIC_EXECUTE,
        GenericAll: FILE_ALL_ACCESS,
    };
    let mut mapped_access = desired_access;
    unsafe { MapGenericMask(&mut mapped_access, &mapping) };
    // File access checks do not normally consume privileges, but AccessCheck
    // requires caller-provided storage and reports the exact size it used.
    let mut privilege_bytes = vec![0_u8; 1024];
    let mut privilege_bytes_len = privilege_bytes.len() as u32;
    let mut granted_access = 0;
    let mut access_status = 0;
    let checked = unsafe {
        AccessCheck(
            descriptor,
            impersonation_token,
            mapped_access,
            &mapping,
            privilege_bytes.as_mut_ptr().cast::<PRIVILEGE_SET>(),
            &mut privilege_bytes_len,
            &mut granted_access,
            &mut access_status,
        )
    };
    unsafe {
        CloseHandle(impersonation_token);
        windows_sys::Win32::Foundation::LocalFree(descriptor.cast());
    }
    if checked == 0 {
        return Err(last_error("stage=access_check AccessCheck"));
    }
    Ok(access_status != 0 && granted_access & mapped_access == mapped_access)
}

const ACL_LEDGER_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PersistentAclKind {
    Read,
    Write,
    DenyRead,
    DenyWrite,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct PersistentAclEntry {
    account: String,
    path: std::path::PathBuf,
    kind: PersistentAclKind,
    #[serde(default)]
    sid: Vec<u8>,
    #[serde(default = "legacy_acl_entry_permissions_version")]
    permissions_version: u32,
}

fn legacy_acl_entry_permissions_version() -> u32 {
    1
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistentAclLedger {
    version: u32,
    entries: Vec<PersistentAclEntry>,
}

impl Default for PersistentAclLedger {
    fn default() -> Self {
        Self {
            version: ACL_LEDGER_VERSION,
            entries: Vec::new(),
        }
    }
}

fn acl_ledger_path() -> std::path::PathBuf {
    crate::setup::state_dir().join("acl-ledger.json")
}

fn load_acl_ledger() -> Result<PersistentAclLedger> {
    let path = acl_ledger_path();
    if !path.exists() {
        return Ok(PersistentAclLedger::default());
    }
    let mut ledger: PersistentAclLedger = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("read ACL ledger {}", path.display()))?,
    )
    .with_context(|| format!("parse ACL ledger {}", path.display()))?;
    if ledger.version != ACL_LEDGER_VERSION {
        anyhow::bail!(
            "unsupported ACL ledger version {} (expected {})",
            ledger.version,
            ACL_LEDGER_VERSION
        )
    }
    ledger.entries.retain(|entry| entry.path.exists());
    Ok(ledger)
}

fn save_acl_ledger(ledger: &PersistentAclLedger) -> Result<()> {
    let path = acl_ledger_path();
    crate::setup::ensure_parent(&path)?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(ledger)?)
        .with_context(|| format!("write ACL ledger temporary file {}", temporary.display()))?;
    let temporary_w = wide(temporary.as_os_str());
    let path_w = wide(path.as_os_str());
    let moved = unsafe {
        MoveFileExW(
            temporary_w.as_ptr(),
            path_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let _ = std::fs::remove_file(&temporary);
        return Err(last_error("publish ACL ledger with MoveFileExW"));
    }
    Ok(())
}

fn verify_persistent_user_permissions(
    request: &SandboxRequest,
    account: &str,
    token: HANDLE,
) -> Result<()> {
    let mut missing = Vec::new();
    for path in request
        .filesystem
        .deny_read
        .iter()
        .filter(|path| path.exists())
    {
        if effective_file_access(token, path, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)? {
            missing.push(format!("deny_read:{}", path.display()));
        }
    }
    for capability in &request.filesystem.read_execute {
        if request
            .filesystem
            .write
            .iter()
            .any(|write_root| path_starts_with(&capability.path, write_root))
        {
            continue;
        }
        if !effective_file_access(
            token,
            &capability.path,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )? {
            let kind = match capability.provisioning {
                ReadProvisioning::Managed => "managed_read",
                ReadProvisioning::ExistingOnly => "external_runtime",
            };
            missing.push(format!("{kind}:{}", capability.path.display()));
        }
    }
    for path in &request.filesystem.write {
        if !effective_file_access(token, path, WORKSPACE_WRITE_PERMISSIONS)? {
            missing.push(format!("managed_write:{}", path.display()));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "stage=provision_acl sandbox account '{account}' is not prepared for this policy scope: {}. Run `opentopia-sandbox provision` for managed roots; external_runtime roots are immutable and must already allow normal-user read/execute access",
        missing.join(", ")
    )
}

fn verify_persistent_capability_permissions(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<()> {
    let desired = capability_acl_entries(request, principal, sid)?;
    let ledger = load_acl_ledger()?;
    let missing = desired
        .iter()
        .filter(|entry| !ledger.entries.contains(entry))
        .map(|entry| format!("{:?}:{}", entry.kind, entry.path.display()))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "stage=provision_acl capability scope '{principal}' is not prepared: {}. Run `opentopia-sandbox provision` before command startup",
        missing.join(", ")
    )
}

fn ensure_persistent_user_permissions(
    request: &SandboxRequest,
    account: &str,
    sid: PSID,
    token: HANDLE,
) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let mut desired = Vec::new();
    for path in request
        .filesystem
        .deny_read
        .iter()
        .filter(|path| path.exists())
    {
        let deny_entry_installed = ledger.entries.iter().any(|entry| {
            entry.account.eq_ignore_ascii_case(account)
                && entry.path == *path
                && entry.kind == PersistentAclKind::DenyRead
        });
        if !deny_entry_installed
            && effective_file_access(token, path, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?
        {
            desired.push((path.clone(), PersistentAclKind::DenyRead));
        }
    }

    let mut inaccessible_external = Vec::new();
    for capability in &request.filesystem.read_execute {
        if request
            .filesystem
            .write
            .iter()
            .any(|write_root| path_starts_with(&capability.path, write_root))
        {
            continue;
        }
        if effective_file_access(
            token,
            &capability.path,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )? {
            crate::logging::event(
                "access_check",
                format!(
                    "satisfied account={account} intent=read_execute path={}",
                    capability.path.display()
                ),
            );
            continue;
        }
        // ExistingOnly is an immutable boundary even when a broad managed
        // root happens to contain it. External SDK/runtime ACLs are never
        // rewritten by OpenTopia.
        match capability.provisioning {
            ReadProvisioning::Managed => {
                desired.push((capability.path.clone(), PersistentAclKind::Read))
            }
            ReadProvisioning::ExistingOnly => inaccessible_external.push(capability.path.clone()),
        }
    }
    if !inaccessible_external.is_empty() {
        let paths = inaccessible_external
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "stage=resolve_runtime runtime_not_accessible sandbox account '{account}' cannot read/execute external runtime roots: {paths}. OpenTopia did not rewrite their host ACLs; provision the runtime for normal-user read access or place it in an OpenTopia-managed runtime location"
        )
    }

    for path in &request.filesystem.write {
        if !effective_file_access(token, path, WORKSPACE_WRITE_PERMISSIONS)? {
            desired.push((path.clone(), PersistentAclKind::Write));
        }
    }
    if desired.is_empty() {
        crate::logging::event(
            "access_check",
            format!("all filesystem access already satisfied for {account}; no ACL mutation"),
        );
        return Ok(());
    }

    let _guards = NamedAclMutex::acquire_paths(desired.iter().map(|(path, _)| path.as_path()))?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let sid_bytes = SidBuffer::copy_from_sid(sid)?.0;
    let mut applied_entries = Vec::new();

    for (path, kind) in desired {
        let entry = PersistentAclEntry {
            account: account.to_string(),
            path: path.clone(),
            kind: kind.clone(),
            sid: sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        };
        revoke_replaced_acl_principals(&ledger, &entry)?;
        if ledger.entries.contains(&entry) {
            continue;
        }
        crate::logging::event(
            "apply_acl",
            format!(
                "applying persistent {:?} permissions for {account} to {}",
                kind,
                path.display()
            ),
        );
        match kind {
            PersistentAclKind::Read => {
                transaction.grant(&path, sid, true, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?
            }
            PersistentAclKind::Write => {
                transaction.grant(&path, sid, true, WORKSPACE_WRITE_PERMISSIONS)?
            }
            PersistentAclKind::DenyRead => {
                transaction.deny(&path, sid, true, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?
            }
            PersistentAclKind::DenyWrite => unreachable!(),
        }
        ledger.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    if !applied_entries.is_empty() {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        let mut latest = load_acl_ledger()?;
        for entry in applied_entries {
            latest.entries.retain(|existing| {
                existing.account != entry.account
                    || existing.path != entry.path
                    || existing.kind != entry.kind
            });
            latest.entries.push(entry);
        }
        save_acl_ledger(&latest)?;
    }
    transaction.commit();
    Ok(())
}

/// Version 2 expressed protected paths with identity-wide deny ACEs. Those
/// denies cannot represent one approval scope, so remove them before switching
/// to capability-scoped deny ACEs. Dedicated-user write ACEs remain the normal
/// side of WRITE_RESTRICTED's two access checks; the capability SID supplies
/// the independent restricted side and prevents ambient writes from widening
/// the target process policy.
fn migrate_legacy_dedicated_user_acls(request: &SandboxRequest, account: &str) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let stale = ledger
        .entries
        .iter()
        .filter(|entry| {
            entry.account.eq_ignore_ascii_case(account)
                && entry.kind == PersistentAclKind::DenyWrite
        })
        .cloned()
        .collect::<Vec<_>>();
    if stale.is_empty() {
        return Ok(());
    }

    let _guards = NamedAclMutex::acquire_paths(stale.iter().map(|entry| entry.path.as_path()))?;
    let mut ledger = load_acl_ledger()?;
    let mut revoked = BTreeSet::new();
    for entry in &stale {
        let mut sid = acl_entry_sid(entry)?;
        let key = (entry.path.clone(), sid.0.clone());
        if revoked.insert(key) {
            update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
    }
    ledger.entries.retain(|entry| {
        !entry.account.eq_ignore_ascii_case(account) || entry.kind != PersistentAclKind::DenyWrite
    });
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest.entries.retain(|entry| !stale.contains(entry));
    save_acl_ledger(&latest)?;
    crate::logging::event(
        "migrate_acl",
        format!(
            "removed {} identity-wide protected-path deny ACL entries for {account}; capability_scope={}",
            stale.len(),
            capability_principal(request)
        ),
    );
    Ok(())
}

fn ensure_persistent_capability_permissions(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<()> {
    let desired_entries = capability_acl_entries(request, principal, sid)?;
    if desired_entries.is_empty() {
        return Ok(());
    }
    if !desired_entries.is_empty() {
        let ledger = load_acl_ledger()?;
        if desired_entries
            .iter()
            .all(|entry| ledger.entries.contains(entry))
        {
            crate::logging::event(
                "access_check",
                format!("capability ACL already provisioned for {principal}"),
            );
            return Ok(());
        }
    }
    let preliminary_ledger = load_acl_ledger()?;
    let legacy_paths = preliminary_ledger
        .entries
        .iter()
        .filter(|entry| entry.account == LEGACY_CAPABILITY_PRINCIPAL)
        .map(|entry| entry.path.as_path())
        .collect::<Vec<_>>();
    let _guards = NamedAclMutex::acquire_paths(
        desired_entries
            .iter()
            .map(|entry| entry.path.as_path())
            .chain(legacy_paths),
    )?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let mut applied_entries = Vec::new();
    let legacy_entries = ledger
        .entries
        .iter()
        .filter(|entry| entry.account == LEGACY_CAPABILITY_PRINCIPAL)
        .cloned()
        .collect::<Vec<_>>();
    if !legacy_entries.is_empty() {
        let mut legacy_sid = SidBuffer::legacy_opentopia_capability();
        for entry in &legacy_entries {
            update_dacl(&entry.path, legacy_sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
        ledger
            .entries
            .retain(|entry| entry.account != LEGACY_CAPABILITY_PRINCIPAL);
    }
    for entry in desired_entries {
        let path = entry.path.clone();
        let kind = entry.kind.clone();
        revoke_replaced_acl_principals(&ledger, &entry)?;
        if ledger.entries.contains(&entry) {
            continue;
        }
        match kind {
            PersistentAclKind::Write => {
                transaction.grant(&path, sid, true, WORKSPACE_WRITE_PERMISSIONS)?
            }
            PersistentAclKind::DenyWrite => transaction.deny_write(&path, sid, true)?,
            PersistentAclKind::Read | PersistentAclKind::DenyRead => unreachable!(),
        }
        ledger.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    if !legacy_entries.is_empty() || !applied_entries.is_empty() {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        let mut latest = load_acl_ledger()?;
        latest
            .entries
            .retain(|entry| entry.account != LEGACY_CAPABILITY_PRINCIPAL);
        for entry in applied_entries {
            latest.entries.retain(|existing| {
                existing.account != entry.account
                    || existing.path != entry.path
                    || existing.kind != entry.kind
            });
            latest.entries.push(entry);
        }
        save_acl_ledger(&latest)?;
    }
    transaction.commit();
    Ok(())
}

fn capability_acl_entries(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<Vec<PersistentAclEntry>> {
    let sid_bytes = SidBuffer::copy_from_sid(sid)?.0;
    let approved = &request.filesystem.allow_protected_write;
    Ok(request
        .filesystem
        .write
        .iter()
        .cloned()
        .map(|path| (path, PersistentAclKind::Write))
        .chain(
            request
                .filesystem
                .deny_write
                .iter()
                .filter(|path| path.exists())
                .filter(|path| {
                    !approved.iter().any(|approved_root| {
                        path_starts_with(path, approved_root)
                            || path_starts_with(approved_root, path)
                    })
                })
                .cloned()
                .map(|path| (path, PersistentAclKind::DenyWrite)),
        )
        .map(|(path, kind)| PersistentAclEntry {
            account: principal.to_string(),
            path,
            kind,
            sid: sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        })
        .collect())
}

fn revoke_replaced_acl_principals(
    ledger: &PersistentAclLedger,
    desired: &PersistentAclEntry,
) -> Result<()> {
    let mut replaced = BTreeSet::new();
    for entry in ledger.entries.iter().filter(|entry| {
        entry.account == desired.account
            && entry.path == desired.path
            && entry.kind == desired.kind
            && !entry.sid.is_empty()
            && entry.sid != desired.sid
    }) {
        if replaced.insert(entry.sid.clone()) {
            let mut sid = SidBuffer(entry.sid.clone());
            update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
    }
    Ok(())
}

pub(super) fn has_dedicated_user_permissions(accounts: &[&str]) -> Result<bool> {
    Ok(load_acl_ledger()?.entries.iter().any(|entry| {
        accounts
            .iter()
            .any(|account| entry.account.eq_ignore_ascii_case(account))
    }))
}

pub(super) fn revoke_dedicated_user_permissions(accounts: &[&str]) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let targets = ledger
        .entries
        .iter()
        .filter(|entry| {
            accounts
                .iter()
                .any(|account| entry.account.eq_ignore_ascii_case(account))
        })
        .cloned()
        .collect::<Vec<_>>();
    let _guards = NamedAclMutex::acquire_paths(targets.iter().map(|entry| entry.path.as_path()))?;
    let mut revoked = BTreeSet::new();
    for entry in &targets {
        let mut sid = acl_entry_sid(entry)?;
        let key = (entry.path.clone(), sid.0.clone());
        if revoked.insert(key) {
            update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
    }
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest.entries.retain(|entry| {
        !accounts
            .iter()
            .any(|account| entry.account.eq_ignore_ascii_case(account))
    });
    save_acl_ledger(&latest)?;
    Ok(())
}

fn acl_entry_sid(entry: &PersistentAclEntry) -> Result<SidBuffer> {
    if entry.sid.is_empty() {
        acl_principal_sid(&entry.account)
    } else {
        Ok(SidBuffer(entry.sid.clone()))
    }
}

pub(super) fn cleanup_workspace_acl(args: &[String]) -> Result<i32> {
    let workspace = match args {
        [flag, value] if flag == "--workspace" => {
            let path = std::path::PathBuf::from(value);
            if !path.is_absolute() || !path.exists() {
                anyhow::bail!("cleanup workspace must be an existing absolute path")
            }
            path.canonicalize()
                .context("canonicalize cleanup workspace")?
        }
        _ => anyhow::bail!("usage: opentopia-sandbox cleanup --workspace <absolute-path>"),
    };
    let ledger = load_acl_ledger()?;
    let targets = ledger
        .entries
        .iter()
        .filter(|entry| path_starts_with(&entry.path, &workspace))
        .cloned()
        .collect::<Vec<_>>();
    let _guards = NamedAclMutex::acquire_paths(targets.iter().map(|entry| entry.path.as_path()))?;
    let mut revoked = BTreeSet::new();
    for entry in targets
        .iter()
        .filter(|entry| revoked.insert((entry.account.clone(), entry.path.clone())))
    {
        let mut sid = acl_entry_sid(entry)?;
        update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
    }
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest
        .entries
        .retain(|entry| !path_starts_with(&entry.path, &workspace));
    save_acl_ledger(&latest)?;
    crate::logging::event(
        "cleanup_acl",
        format!(
            "workspace={} revoked={}",
            workspace.display(),
            revoked.len()
        ),
    );
    println!(
        "OpenTopia sandbox ACL cleanup complete: workspace={} entries={}",
        workspace.display(),
        revoked.len()
    );
    Ok(0)
}

fn path_starts_with(path: &Path, root: &Path) -> bool {
    let path = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let root = root
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let path = path.strip_prefix("\\\\?\\").unwrap_or(&path);
    let root = root.strip_prefix("\\\\?\\").unwrap_or(&root);
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn account_sid(account: &str) -> Result<SidBuffer> {
    let account = wide(OsStr::new(account));
    let mut sid_len = 0;
    let mut domain_len = 0;
    let mut use_type = 0;
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &mut sid_len,
            ptr::null_mut(),
            &mut domain_len,
            &mut use_type,
        );
    }
    if sid_len == 0 {
        return Err(last_error("stage=prepare_sandbox LookupAccountNameW(size)"));
    }
    let mut sid = vec![0_u8; sid_len as usize];
    let mut domain = vec![0_u16; domain_len as usize];
    let found = unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut use_type,
        )
    };
    if found == 0 {
        return Err(last_error("stage=prepare_sandbox LookupAccountNameW"));
    }
    sid.truncate(sid_len as usize);
    Ok(SidBuffer(sid))
}

fn acl_principal_sid(principal: &str) -> Result<SidBuffer> {
    if principal == LEGACY_CAPABILITY_PRINCIPAL {
        Ok(SidBuffer::legacy_opentopia_capability())
    } else if let Some(value) = principal
        .strip_prefix(CAPABILITY_PRINCIPAL_PREFIX)
        .or_else(|| principal.strip_prefix(LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX))
    {
        let id = Uuid::parse_str(value).context("parse scoped capability principal")?;
        Ok(SidBuffer::opentopia_capability(id))
    } else {
        account_sid(principal)
    }
}

fn capability_principal(request: &SandboxRequest) -> String {
    let mut roots = request
        .filesystem
        .write
        .iter()
        .map(|path| normalized_capability_path(path))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        roots.push(normalized_capability_path(&request.cwd));
    }
    roots.extend(
        request
            .filesystem
            .allow_protected_write
            .iter()
            .map(|path| format!("allow:{}", normalized_capability_path(path))),
    );
    roots.extend(
        request
            .filesystem
            .deny_write
            .iter()
            .map(|path| format!("deny:{}", normalized_capability_path(path))),
    );
    roots.sort_unstable();
    roots.dedup();
    let scope = roots.join("\0");
    let namespace = Uuid::from_u128(CAPABILITY_NAMESPACE);
    let id = Uuid::new_v5(&namespace, scope.as_bytes());
    format!("{CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple())
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

struct RestrictedToken {
    handle: HANDLE,
}

impl RestrictedToken {
    fn for_capability(capability_sid: PSID) -> Result<Self> {
        Self::create(capability_sid, None)
    }

    fn for_dedicated_capability(capability_sid: PSID) -> Result<Self> {
        let mut runtime_sid = SidBuffer::opentopia_runtime();
        Self::create(capability_sid, Some(runtime_sid.as_ptr()))
    }

    fn create(capability_sid: PSID, runtime_sid: Option<PSID>) -> Result<Self> {
        let mut base_token = ptr::null_mut();
        let opened = unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ASSIGN_PRIMARY
                    | TOKEN_DUPLICATE
                    | TOKEN_QUERY
                    | TOKEN_ADJUST_DEFAULT
                    | TOKEN_ADJUST_PRIVILEGES
                    | TOKEN_ADJUST_SESSIONID,
                &mut base_token,
            )
        };
        if opened == 0 {
            return Err(last_error("OpenProcessToken"));
        }

        // WRITE_RESTRICTED makes Windows require an allow ACE for this
        // per-invocation capability SID on every write. The ordinary user
        // token may still read OS dependencies, but it cannot use its normal
        // ACL grants to write outside the capability roots.
        let mut logon_sid = match current_logon_sid(base_token) {
            Ok(sid) => sid,
            Err(error) => {
                unsafe { CloseHandle(base_token) };
                return Err(error);
            }
        };
        let mut everyone_sid = SidBuffer::well_known(WIN_WORLD_SID)?;
        let mut restricting_sids = [unsafe { std::mem::zeroed::<SID_AND_ATTRIBUTES>() }; 4];
        restricting_sids[0].Sid = capability_sid;
        let restricting_sid_count = if let Some(runtime_sid) = runtime_sid {
            restricting_sids[1].Sid = runtime_sid;
            restricting_sids[2].Sid = logon_sid.as_ptr();
            restricting_sids[3].Sid = everyone_sid.as_ptr();
            4
        } else {
            restricting_sids[1].Sid = logon_sid.as_ptr();
            restricting_sids[2].Sid = everyone_sid.as_ptr();
            3
        };
        // The dedicated account is already a non-administrator. Preserve its
        // standard SeChangeNotifyPrivilege so runtimes can traverse parent
        // directories on the way to explicitly granted roots. Removing every
        // privilege causes CLR, Git, Python, and Node loaders to terminate with
        // STATUS_DLL_INIT_FAILED before main. WRITE_RESTRICTED remains the
        // independent filesystem-write boundary.
        let flags = if runtime_sid.is_some() {
            WRITE_RESTRICTED
        } else {
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED
        };
        let mut restricted = ptr::null_mut();
        let created = unsafe {
            CreateRestrictedToken(
                base_token,
                flags,
                0,
                ptr::null(),
                0,
                ptr::null(),
                restricting_sid_count,
                restricting_sids.as_ptr(),
                &mut restricted,
            )
        };
        unsafe { CloseHandle(base_token) };
        if created == 0 || restricted.is_null() {
            return Err(last_error("CreateRestrictedToken"));
        }
        // Dedicated runtimes need the logon and world compatibility SIDs for
        // session kernel objects and Windows pseudo devices. This means the
        // dedicated backend is not a complete host-wide write allowlist; the
        // per-scope SID still makes explicit protected-root deny ACEs effective.
        if let Some(runtime_sid) = runtime_sid {
            if let Err(error) = augment_default_dacl(
                restricted,
                &[
                    capability_sid,
                    runtime_sid,
                    logon_sid.as_ptr(),
                    everyone_sid.as_ptr(),
                ],
            ) {
                unsafe { CloseHandle(restricted) };
                return Err(error);
            }
        } else {
            let default_dacl_sids = [capability_sid, logon_sid.as_ptr(), everyone_sid.as_ptr()];
            if let Err(error) = set_default_dacl(restricted, &default_dacl_sids) {
                unsafe { CloseHandle(restricted) };
                return Err(error);
            }
        }
        Ok(Self { handle: restricted })
    }
}

impl Drop for RestrictedToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

fn augment_default_dacl(token: HANDLE, sids: &[PSID]) -> Result<()> {
    let mut needed = 0_u32;
    unsafe { GetTokenInformation(token, TokenDefaultDacl, ptr::null_mut(), 0, &mut needed) };
    anyhow::ensure!(
        needed as usize >= std::mem::size_of::<TokenDefaultDaclInfo>(),
        "GetTokenInformation(TokenDefaultDacl) returned an invalid size"
    );
    let mut buffer = vec![0_u8; needed as usize];
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenDefaultDacl,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if queried == 0 {
        return Err(last_error("GetTokenInformation(TokenDefaultDacl)"));
    }
    let existing = unsafe {
        std::ptr::read_unaligned(buffer.as_ptr().cast::<TokenDefaultDaclInfo>()).default_dacl
    };
    let entries = sids
        .iter()
        .map(|sid| EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN_VALUE,
                ptstrName: sid.cast(),
            },
        })
        .collect::<Vec<_>>();
    let mut augmented = ptr::null_mut();
    let assembled = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            existing,
            &mut augmented,
        )
    };
    if assembled != 0 {
        anyhow::bail!("SetEntriesInAclW for augmented token DACL failed: {assembled}")
    }
    let mut info = TokenDefaultDaclInfo {
        default_dacl: augmented,
    };
    let applied = unsafe {
        SetTokenInformation(
            token,
            TokenDefaultDacl,
            (&mut info as *mut TokenDefaultDaclInfo).cast(),
            std::mem::size_of::<TokenDefaultDaclInfo>() as u32,
        )
    };
    if !augmented.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(augmented.cast()) };
    }
    if applied == 0 {
        return Err(last_error(
            "SetTokenInformation(augmented TokenDefaultDacl)",
        ));
    }
    Ok(())
}

fn set_default_dacl(token: HANDLE, sids: &[PSID]) -> Result<()> {
    let entries = sids
        .iter()
        .map(|sid| EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN_VALUE,
                ptstrName: sid.cast(),
            },
        })
        .collect::<Vec<_>>();
    let mut dacl = ptr::null_mut();
    let assembled = unsafe {
        SetEntriesInAclW(
            entries.len() as u32,
            entries.as_ptr(),
            ptr::null(),
            &mut dacl,
        )
    };
    if assembled != 0 {
        anyhow::bail!("SetEntriesInAclW for token default DACL failed: {assembled}")
    }
    let mut info = TokenDefaultDaclInfo { default_dacl: dacl };
    let applied = unsafe {
        SetTokenInformation(
            token,
            TokenDefaultDacl,
            (&mut info as *mut TokenDefaultDaclInfo).cast(),
            std::mem::size_of::<TokenDefaultDaclInfo>() as u32,
        )
    };
    if !dacl.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(dacl.cast()) };
    }
    if applied == 0 {
        return Err(last_error("SetTokenInformation(TokenDefaultDacl)"));
    }
    Ok(())
}

struct SidBuffer(Vec<u8>);

impl SidBuffer {
    fn opentopia_capability(id: Uuid) -> Self {
        // A stable, scope-specific restricting SID lets the helper install an
        // authorized root set once without making that grant usable by a
        // sandbox launched for a different workspace or approval scope.
        let bytes = *id.as_bytes();
        let values = [
            21_u32,
            u32::from_le_bytes(bytes[0..4].try_into().expect("uuid segment")),
            u32::from_le_bytes(bytes[4..8].try_into().expect("uuid segment")),
            u32::from_le_bytes(bytes[8..12].try_into().expect("uuid segment")),
            u32::from_le_bytes(bytes[12..16].try_into().expect("uuid segment")),
        ];
        let mut bytes = Vec::with_capacity(8 + values.len() * 4);
        bytes.extend([1, values.len() as u8, 0, 0, 0, 0, 0, 5]);
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
        Self(bytes)
    }

    fn legacy_opentopia_capability() -> Self {
        Self::opentopia_capability(Uuid::from_bytes(*b"OpenTopiaSandbox"))
    }

    fn opentopia_runtime() -> Self {
        Self::opentopia_capability(Uuid::from_bytes(*b"OpenTopiaRuntime"))
    }

    fn well_known(kind: i32) -> Result<Self> {
        let mut bytes = vec![0; 68];
        let mut size = bytes.len() as u32;
        let ok = unsafe {
            CreateWellKnownSid(kind, ptr::null_mut(), bytes.as_mut_ptr().cast(), &mut size)
        };
        if ok == 0 {
            return Err(last_error("CreateWellKnownSid"));
        }
        bytes.truncate(size as usize);
        Ok(Self(bytes))
    }

    fn copy_from_sid(sid: PSID) -> Result<Self> {
        let size = unsafe { GetLengthSid(sid) };
        if size == 0 {
            return Err(last_error("GetLengthSid"));
        }
        let mut bytes = vec![0; size as usize];
        let copied = unsafe { CopySid(size, bytes.as_mut_ptr().cast(), sid) };
        if copied == 0 {
            return Err(last_error("CopySid"));
        }
        Ok(Self(bytes))
    }

    fn as_ptr(&mut self) -> PSID {
        self.0.as_mut_ptr().cast()
    }
}

fn current_logon_sid(token: HANDLE) -> Result<SidBuffer> {
    let mut needed = 0;
    unsafe { GetTokenInformation(token, TokenGroups, ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(last_error("GetTokenInformation(TokenGroups)"));
    }
    let mut buffer = vec![0_u8; needed as usize];
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenGroups,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    };
    if queried == 0 || (needed as usize) < std::mem::size_of::<u32>() {
        return Err(last_error("GetTokenInformation(TokenGroups)"));
    }

    let count = unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast::<u32>()) } as usize;
    let first = unsafe { buffer.as_ptr().add(std::mem::size_of::<u32>()) } as usize;
    let alignment = std::mem::align_of::<SID_AND_ATTRIBUTES>();
    let aligned = (first + alignment - 1) & !(alignment - 1);
    let groups = aligned as *const SID_AND_ATTRIBUTES;
    for index in 0..count {
        let group = unsafe { std::ptr::read_unaligned(groups.add(index)) };
        if group.Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID {
            return SidBuffer::copy_from_sid(group.Sid);
        }
    }
    anyhow::bail!("TokenGroups did not include a logon SID")
}

struct AclTransaction {
    changes: Vec<AclChange>,
    journal_path: std::path::PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AclChange {
    path: std::path::PathBuf,
    sid: Vec<u8>,
}

impl Default for AclTransaction {
    fn default() -> Self {
        Self {
            changes: Vec::new(),
            journal_path: crate::setup::state_dir()
                .join("acl-transactions")
                .join(format!("{}.json", Uuid::new_v4().simple())),
        }
    }
}

impl AclTransaction {
    fn grant(&mut self, path: &Path, sid: PSID, inherit: bool, permissions: u32) -> Result<()> {
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid: SidBuffer::copy_from_sid(sid)?.0,
        });
        self.persist()?;
        apply_dacl_change(path, sid, GRANT_ACCESS, inherit, permissions)?;
        Ok(())
    }

    fn deny_write(&mut self, path: &Path, sid: PSID, inherit: bool) -> Result<()> {
        self.deny(path, sid, inherit, WRITE_RESTRICTION_PERMISSIONS)
    }

    fn deny(&mut self, path: &Path, sid: PSID, inherit: bool, permissions: u32) -> Result<()> {
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid: SidBuffer::copy_from_sid(sid)?.0,
        });
        self.persist()?;
        apply_dacl_change(path, sid, DENY_ACCESS, inherit, permissions)?;
        Ok(())
    }

    fn commit(mut self) {
        self.changes.clear();
        let _ = std::fs::remove_file(&self.journal_path);
    }

    fn persist(&self) -> Result<()> {
        crate::setup::ensure_parent(&self.journal_path)?;
        std::fs::write(
            &self.journal_path,
            serde_json::to_vec_pretty(&self.changes)?,
        )
        .with_context(|| format!("write ACL recovery journal {}", self.journal_path.display()))
    }
}

impl Drop for AclTransaction {
    fn drop(&mut self) {
        let mut recovered = true;
        for change in self.changes.iter().rev() {
            let mut sid = SidBuffer(change.sid.clone());
            recovered &= update_dacl(&change.path, sid.as_ptr(), REVOKE_ACCESS, false, 0).is_ok();
        }
        if recovered {
            let _ = std::fs::remove_file(&self.journal_path);
        }
    }
}

fn recover_acl_transactions() -> Result<()> {
    let directory = crate::setup::state_dir().join("acl-transactions");
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&directory)
        .with_context(|| format!("read ACL transaction directory {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let changes: Vec<AclChange> = serde_json::from_slice(
            &std::fs::read(&path)
                .with_context(|| format!("read ACL recovery journal {}", path.display()))?,
        )
        .with_context(|| format!("parse ACL recovery journal {}", path.display()))?;
        let _guards =
            NamedAclMutex::acquire_paths(changes.iter().map(|change| change.path.as_path()))?;
        for change in changes.iter().rev() {
            let mut sid = SidBuffer(change.sid.clone());
            apply_dacl_change(&change.path, sid.as_ptr(), REVOKE_ACCESS, false, 0).with_context(
                || format!("recover interrupted ACL transaction {}", path.display()),
            )?;
        }
        std::fs::remove_file(&path)
            .with_context(|| format!("remove recovered ACL journal {}", path.display()))?;
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AclFailureCacheEntry {
    recorded_at_ms: u128,
    message: String,
}

fn apply_dacl_change(
    path: &Path,
    sid: PSID,
    access_mode: i32,
    inherit: bool,
    permissions: u32,
) -> Result<()> {
    const FAILURE_TTL_MS: u128 = 5 * 60 * 1_000;
    const FAILURE_NAMESPACE: u128 = 0xf023_7719_ae79_5a4f_ba15_73e0_3820_d3fd;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id = Uuid::new_v5(
        &Uuid::from_u128(FAILURE_NAMESPACE),
        normalized_capability_path(path).as_bytes(),
    );
    let cache_path = crate::setup::state_dir()
        .join("acl-failures")
        .join(format!("{}.json", id.simple()));
    if let Ok(bytes) = std::fs::read(&cache_path) {
        if let Ok(cached) = serde_json::from_slice::<AclFailureCacheEntry>(&bytes) {
            if now.saturating_sub(cached.recorded_at_ms) < FAILURE_TTL_MS {
                anyhow::bail!(
                    "stage=apply_acl cached deterministic ACL failure for {}: {}",
                    path.display(),
                    cached.message
                );
            }
        }
        let _ = std::fs::remove_file(&cache_path);
    }
    match update_dacl(path, sid, access_mode, inherit, permissions) {
        Ok(()) => {
            let _ = std::fs::remove_file(&cache_path);
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            if message.contains(": 5") {
                crate::setup::ensure_parent(&cache_path)?;
                let cached = AclFailureCacheEntry {
                    recorded_at_ms: now,
                    message: message.clone(),
                };
                std::fs::write(&cache_path, serde_json::to_vec_pretty(&cached)?)
                    .with_context(|| format!("write ACL failure cache {}", cache_path.display()))?;
            }
            Err(error)
        }
    }
}

fn update_dacl(
    path: &Path,
    sid: PSID,
    access_mode: i32,
    inherit: bool,
    permissions: u32,
) -> Result<()> {
    let name = wide(path.as_os_str());
    let mut old_dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let get_result = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if get_result != 0 {
        anyhow::bail!(
            "GetNamedSecurityInfoW failed for {}: {get_result}",
            path.display()
        )
    }
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: permissions,
        grfAccessMode: access_mode,
        grfInheritance: if inherit && path.is_dir() {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            0
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN_VALUE,
            ptstrName: sid.cast(),
        },
    };
    let mut new_dacl = ptr::null_mut();
    let entries_result = unsafe { SetEntriesInAclW(1, &entry, old_dacl, &mut new_dacl) };
    if !descriptor.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(descriptor.cast()) };
    }
    if entries_result != 0 {
        anyhow::bail!(
            "SetEntriesInAclW failed for {}: {entries_result}",
            path.display()
        )
    }
    let set_result = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
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
    if set_result != 0 {
        anyhow::bail!(
            "SetNamedSecurityInfoW failed for {}: {set_result}",
            path.display()
        )
    }
    Ok(())
}

struct NamedAclMutex(HANDLE);

impl NamedAclMutex {
    fn acquire_named(name: &str, timeout_ms: u32, purpose: &str) -> Result<Self> {
        let name = wide(OsStr::new(name));
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(last_error("stage=apply_acl CreateMutexW"));
        }
        let waited = unsafe { WaitForSingleObject(handle, timeout_ms) };
        if waited == WAIT_TIMEOUT {
            unsafe { CloseHandle(handle) };
            anyhow::bail!(
                "stage=apply_acl timed out waiting for the {purpose} lock after {timeout_ms}ms"
            )
        }
        if waited == u32::MAX {
            unsafe { CloseHandle(handle) };
            return Err(last_error("stage=apply_acl WaitForSingleObject"));
        }
        Ok(Self(handle))
    }

    fn acquire_paths<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Result<Vec<Self>> {
        const ACL_LOCK_NAMESPACE: u128 = 0x342a_f2f8_a5e0_5920_99f1_ec1b_c7c5_74e0;
        let namespace = Uuid::from_u128(ACL_LOCK_NAMESPACE);
        let mut names = paths
            .into_iter()
            .map(acl_authorization_domain)
            .map(|domain| {
                let id = Uuid::new_v5(&namespace, domain.as_bytes());
                format!("Local\\OpenTopiaSandboxAclScope-{}", id.simple())
            })
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
            .iter()
            .map(|name| Self::acquire_named(name, 120_000, "ACL authorization-domain"))
            .collect()
    }

    fn acquire_metadata() -> Result<Self> {
        Self::acquire_named(
            "Local\\OpenTopiaSandboxAclLedger",
            5_000,
            "ACL ledger transaction",
        )
    }
}

fn acl_authorization_domain(path: &Path) -> String {
    let normalized = normalized_capability_path(path);
    let parts = normalized
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if normalized.starts_with("\\\\") {
        return format!(
            "\\\\{}\\{}",
            parts.first().copied().unwrap_or_default(),
            parts.get(1).copied().unwrap_or_default()
        );
    }
    match (parts.first(), parts.get(1)) {
        (Some(volume), Some(top_level)) => format!("{volume}\\{top_level}"),
        (Some(volume), None) => (*volume).to_string(),
        _ => normalized,
    }
}

impl Drop for NamedAclMutex {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

fn launch(request: &SandboxRequest, restricted_token: HANDLE) -> Result<u32> {
    let job = create_job(request)?;
    let mut attributes = AttributeList::new(2)?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
        (&job as *const HANDLE).cast(),
        std::mem::size_of::<HANDLE>(),
    )?;
    let handles = stdio_handles()?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
        handles.inherited.as_ptr().cast(),
        std::mem::size_of_val(handles.inherited.as_slice()),
    )?;

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    let mut desktop = wide("winsta0\\default");
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.lpDesktop = desktop.as_mut_ptr();
    startup.StartupInfo.hStdInput = handles.stdio[0];
    startup.StartupInfo.hStdOutput = handles.stdio[1];
    startup.StartupInfo.hStdError = handles.stdio[2];
    startup.lpAttributeList = attributes.as_mut_ptr();

    let mut command_line = wide(argv_to_command_line(&request.command));
    let cwd = wide(native_path(&request.cwd).as_os_str());
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let env_block = current_environment_block(Some(&request.cwd), None, false);
    let mut creation_flags = CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT;
    if !request.interactive {
        creation_flags |= CREATE_NO_WINDOW;
    }
    let created = unsafe {
        CreateProcessAsUserW(
            restricted_token,
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            creation_flags,
            env_block.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process_info,
        )
    };
    if created == 0 {
        unsafe { CloseHandle(job) };
        return Err(last_error("CreateProcessAsUserW"));
    }
    unsafe { CloseHandle(process_info.hThread) };
    let timeout = request
        .timeout_ms
        .map(|value| value.min(u32::MAX as u64) as u32)
        .unwrap_or(INFINITE);
    let waited = unsafe { WaitForSingleObject(process_info.hProcess, timeout) };
    if waited == WAIT_TIMEOUT {
        unsafe { TerminateJobObject(job, 124) };
        let termination_timeout = request.termination_timeout_ms.min(u32::MAX as u64) as u32;
        let terminated = unsafe { WaitForSingleObject(process_info.hProcess, termination_timeout) };
        unsafe {
            CloseHandle(process_info.hProcess);
            CloseHandle(job);
        }
        if terminated == WAIT_TIMEOUT {
            anyhow::bail!(
                "stage=terminate command timed out after {}ms and the process tree did not exit within {}ms",
                request.timeout_ms.unwrap_or_default(),
                request.termination_timeout_ms
            )
        }
        anyhow::bail!(
            "stage=wait command timed out after {}ms; the process tree was terminated",
            request.timeout_ms.unwrap_or_default()
        )
    }
    if waited == u32::MAX {
        unsafe {
            CloseHandle(process_info.hProcess);
            CloseHandle(job);
        }
        return Err(last_error("stage=wait WaitForSingleObject"));
    }
    let mut exit_code = 1;
    let code = unsafe { GetExitCodeProcess(process_info.hProcess, &mut exit_code) };
    unsafe {
        CloseHandle(process_info.hProcess);
        CloseHandle(job);
    }
    if code == 0 {
        return Err(last_error("stage=wait GetExitCodeProcess"));
    }
    Ok(exit_code)
}

fn create_job(request: &SandboxRequest) -> Result<HANDLE> {
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if let Some(bytes) = request.max_memory_bytes {
        flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
        limits.JobMemoryLimit = usize::try_from(bytes)
            .context("stage=prepare_sandbox memory limit exceeds platform range")?;
    }
    if let Some(milliseconds) = request.max_cpu_time_ms {
        flags |= JOB_OBJECT_LIMIT_JOB_TIME;
        limits.BasicLimitInformation.PerJobUserTimeLimit = milliseconds
            .checked_mul(10_000)
            .and_then(|value| i64::try_from(value).ok())
            .context("stage=prepare_sandbox CPU time limit exceeds platform range")?;
    }
    limits.BasicLimitInformation.LimitFlags = flags;
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() || job == INVALID_HANDLE_VALUE {
        return Err(last_error("CreateJobObjectW"));
    }
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        unsafe { CloseHandle(job) };
        return Err(last_error("SetInformationJobObject"));
    }
    Ok(job)
}

struct StdioHandles {
    stdio: [HANDLE; 3],
    inherited: Vec<HANDLE>,
}

fn stdio_handles() -> Result<StdioHandles> {
    let stdio = [
        unsafe { GetStdHandle(STD_INPUT_HANDLE) },
        unsafe { GetStdHandle(STD_OUTPUT_HANDLE) },
        unsafe { GetStdHandle(STD_ERROR_HANDLE) },
    ];
    if stdio
        .iter()
        .any(|handle| handle.is_null() || *handle == INVALID_HANDLE_VALUE)
    {
        return Err(last_error("GetStdHandle"));
    }
    for handle in stdio.iter().copied() {
        let inherited =
            unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
        if inherited == 0 {
            return Err(last_error("SetHandleInformation(stdio)"));
        }
    }
    let mut unique = BTreeSet::new();
    unique.extend(stdio);
    Ok(StdioHandles {
        stdio,
        inherited: unique.into_iter().collect(),
    })
}

struct AttributeList {
    storage: Vec<u8>,
}

impl AttributeList {
    fn new(count: u32) -> Result<Self> {
        let mut size = 0;
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size) };
        let mut storage = vec![0u8; size];
        let list = storage.as_mut_ptr().cast::<std::ffi::c_void>();
        let initialized =
            unsafe { InitializeProcThreadAttributeList(list.cast::<_>(), count, 0, &mut size) };
        if initialized == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        Ok(Self { storage })
    }

    fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    fn update(
        &mut self,
        attribute: usize,
        value: *const std::ffi::c_void,
        size: usize,
    ) -> Result<()> {
        let updated = unsafe {
            UpdateProcThreadAttribute(
                self.as_mut_ptr(),
                0,
                attribute,
                value,
                size,
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            return Err(last_error("UpdateProcThreadAttribute"));
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) }
    }
}

fn argv_to_command_line(argv: &[String]) -> String {
    argv.iter()
        .map(|argument| quote_windows_arg(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_windows_arg(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|character| matches!(character, ' ' | '\t' | '\n' | '\r' | '"'))
    {
        return argument.to_owned();
    }
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(character);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn native_path(path: &Path) -> std::path::PathBuf {
    let display = path.as_os_str().to_string_lossy();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(native) = display.strip_prefix(r"\\?\") {
        return std::path::PathBuf::from(native);
    }
    path.to_path_buf()
}

fn last_error(operation: &str) -> anyhow::Error {
    anyhow::anyhow!("{operation} failed: {}", unsafe { GetLastError() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_prefix_matching_respects_component_boundaries_and_extended_paths() {
        assert!(path_starts_with(
            Path::new(r"\\?\C:\workspace\nested"),
            Path::new(r"C:\workspace")
        ));
        assert!(!path_starts_with(
            Path::new(r"C:\workspace-other"),
            Path::new(r"C:\workspace")
        ));
    }

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
    fn dedicated_restricted_token_keeps_runtime_compatibility_sids_explicit() {
        let source = include_str!("windows.rs");
        let function = source
            .split("impl RestrictedToken")
            .nth(1)
            .expect("restricted token implementation")
            .split("impl Drop for RestrictedToken")
            .next()
            .expect("restricted token boundary");
        assert!(function.contains("restricting_sids[0].Sid = capability_sid"));
        assert!(function.contains("restricting_sids[1].Sid = runtime_sid"));
        assert!(function.contains("restricting_sids[2].Sid = logon_sid.as_ptr()"));
        assert!(function.contains("restricting_sids[3].Sid = everyone_sid.as_ptr()"));
        assert!(!function.contains("ensure_runtime_registry_access"));
        assert!(function.contains("augment_default_dacl"));
        assert!(!function.contains("current_user_sid"));
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
    fn acl_locks_are_sharded_by_authorization_domain() {
        assert_eq!(
            acl_authorization_domain(Path::new(r"J:\Project\OpenTopia")),
            r"j:\project"
        );
        assert_eq!(
            acl_authorization_domain(Path::new(r"J:\Python311")),
            r"j:\python311"
        );
        assert_ne!(
            acl_authorization_domain(Path::new(r"J:\Project\OpenTopia")),
            acl_authorization_domain(Path::new(r"J:\Python311"))
        );
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

    #[test]
    fn filesystem_capability_principal_is_stable_and_scope_specific() {
        let request = capability_request(&[r"C:\workspace", r"C:\sandbox-home"]);
        let reordered = capability_request(&[r"C:\sandbox-home", r"C:\workspace"]);
        let other = capability_request(&[r"C:\other-workspace", r"C:\sandbox-home"]);
        let principal = capability_principal(&request);
        assert_eq!(principal, capability_principal(&reordered));
        assert_ne!(principal, capability_principal(&other));

        let first = acl_principal_sid(&principal).expect("resolve scoped capability principal");
        let second = acl_principal_sid(&capability_principal(&request))
            .expect("resolve stable capability principal");
        assert_eq!(first.0, second.0);
    }

    #[test]
    fn legacy_scoped_capability_principals_remain_resolvable_for_cleanup() {
        let id = Uuid::new_v4();
        let old = format!("{LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple());
        let new = format!("{CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple());
        assert_eq!(
            acl_principal_sid(&old).expect("legacy SID").0,
            acl_principal_sid(&new).expect("current SID").0
        );
    }

    #[test]
    fn native_path_removes_win32_extended_prefixes_for_process_creation() {
        assert_eq!(
            native_path(Path::new(r"\\?\J:\workspace")),
            Path::new(r"J:\workspace")
        );
        assert_eq!(
            native_path(Path::new(r"\\?\UNC\server\share\workspace")),
            Path::new(r"\\server\share\workspace")
        );
    }

    fn capability_request(write_roots: &[&str]) -> SandboxRequest {
        SandboxRequest {
            interactive: false,
            cwd: Path::new(r"C:\workspace").to_path_buf(),
            filesystem: opentopia_sandbox_protocol::FilesystemCapabilities {
                write: write_roots.iter().map(std::path::PathBuf::from).collect(),
                ..Default::default()
            },
            network: NetworkMode::Internet,
            timeout_ms: Some(1_000),
            termination_timeout_ms: 500,
            max_memory_bytes: None,
            max_cpu_time_ms: None,
            max_output_bytes: None,
            backend: BackendMode::Unelevated,
            command: vec!["cmd.exe".to_string()],
        }
    }
}
