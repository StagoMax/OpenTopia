use super::BackendMode;
use super::NetworkMode;
use super::SandboxRequest;
use crate::process_env::current_environment_block;
use anyhow::Context;
use anyhow::Result;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
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
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::SetEntriesInAclW;
use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::DENY_ACCESS;
use windows_sys::Win32::Security::Authorization::EXPLICIT_ACCESS_W;
use windows_sys::Win32::Security::Authorization::GRANT_ACCESS;
use windows_sys::Win32::Security::Authorization::REVOKE_ACCESS;
use windows_sys::Win32::Security::Authorization::SE_FILE_OBJECT;
use windows_sys::Win32::Security::Authorization::TRUSTEE_IS_SID;
use windows_sys::Win32::Security::Authorization::TRUSTEE_IS_UNKNOWN;
use windows_sys::Win32::Security::Authorization::TRUSTEE_W;
use windows_sys::Win32::Security::CopySid;
use windows_sys::Win32::Security::CreateRestrictedToken;
use windows_sys::Win32::Security::CreateWellKnownSid;
use windows_sys::Win32::Security::GetLengthSid;
use windows_sys::Win32::Security::GetTokenInformation;
use windows_sys::Win32::Security::LookupAccountNameW;
use windows_sys::Win32::Security::SetTokenInformation;
use windows_sys::Win32::Security::TokenDefaultDacl;
use windows_sys::Win32::Security::TokenGroups;
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::Security::DISABLE_MAX_PRIVILEGE;
use windows_sys::Win32::Security::LUA_TOKEN;
use windows_sys::Win32::Security::PSID;
use windows_sys::Win32::Security::SID_AND_ATTRIBUTES;
use windows_sys::Win32::Security::SUB_CONTAINERS_AND_OBJECTS_INHERIT;
use windows_sys::Win32::Security::TOKEN_ADJUST_DEFAULT;
use windows_sys::Win32::Security::TOKEN_ADJUST_PRIVILEGES;
use windows_sys::Win32::Security::TOKEN_ADJUST_SESSIONID;
use windows_sys::Win32::Security::TOKEN_ASSIGN_PRIMARY;
use windows_sys::Win32::Security::TOKEN_DUPLICATE;
use windows_sys::Win32::Security::TOKEN_QUERY;
use windows_sys::Win32::Security::WRITE_RESTRICTED;
use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
use windows_sys::Win32::Storage::FileSystem::DELETE;
use windows_sys::Win32::Storage::FileSystem::FILE_DELETE_CHILD;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_EXECUTE;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_WRITE;
use windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING;
use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
use windows_sys::Win32::System::Console::GetStdHandle;
use windows_sys::Win32::System::Console::STD_ERROR_HANDLE;
use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;
use windows_sys::Win32::System::Console::STD_OUTPUT_HANDLE;
use windows_sys::Win32::System::JobObjects::CreateJobObjectW;
use windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation;
use windows_sys::Win32::System::JobObjects::SetInformationJobObject;
use windows_sys::Win32::System::JobObjects::TerminateJobObject;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_JOB_MEMORY;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_JOB_TIME;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::System::Threading::CreateProcessAsUserW;
use windows_sys::Win32::System::Threading::CreateProcessW;
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
use windows_sys::Win32::UI::Shell::GetUserProfileDirectoryW;
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
    crate::logging::event(
        "backend_select",
        format!(
            "requested={:?} network={:?}",
            request.backend, request.network
        ),
    );
    match request.backend {
        BackendMode::Elevated if request.interactive => anyhow::bail!(
            "stage=validate_policy interactive PTY sessions currently require the unelevated Windows sandbox backend"
        ),
        BackendMode::Auto if request.interactive && crate::setup::is_complete() => anyhow::bail!(
            "stage=validate_policy interactive PTY sessions currently require the unelevated Windows sandbox backend"
        ),
        BackendMode::Unelevated if crate::setup::is_complete() => anyhow::bail!(
            "stage=validate_policy unelevated execution is disabled after elevated setup because it shares the host identity; use auto or elevated"
        ),
        BackendMode::Unelevated if !request.denied_read_paths.is_empty() => anyhow::bail!(
            "stage=validate_policy unelevated backend cannot enforce deny-read requirements"
        ),
        BackendMode::Unelevated => run_unelevated(request),
        BackendMode::Elevated => run_elevated(request),
        BackendMode::Auto if crate::setup::is_complete() => run_elevated(request),
        BackendMode::Auto if !request.denied_read_paths.is_empty() => anyhow::bail!(
            "stage=validate_policy deny-read requirements need completed elevated sandbox setup"
        ),
        BackendMode::Auto => run_unelevated(request),
    }
}

fn run_unelevated(request: SandboxRequest) -> Result<i32> {
    crate::logging::event(
        "prepare_sandbox",
        "starting unelevated WRITE_RESTRICTED backend",
    );
    if request.network == NetworkMode::Deny {
        anyhow::bail!(
            "stage=validate_policy unelevated backend cannot authoritatively enforce offline networking; run elevated setup or allow network"
        )
    }
    let capability_principal = capability_principal(&request);
    let mut capability = acl_principal_sid(&capability_principal)?;
    let capability_sid = capability.as_ptr();
    ensure_persistent_capability_permissions(&request, &capability_principal, capability_sid)
        .context("stage=apply_acl ensure capability permissions")?;
    let _protected_write_lock = (!request.allowed_protected_roots.is_empty())
        .then(NamedAclMutex::acquire)
        .transpose()?;
    let _protected_write_window =
        ProtectedWriteWindow::open(request.allowed_protected_roots.clone(), capability_sid)?;

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
            request.write_roots.len(),
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
struct ElevatedRunnerResult {
    exit_code: i32,
    error: Option<String>,
}

const ELEVATED_RUNNER_PROTOCOL_VERSION: u32 = 1;
const LEGACY_UNELEVATED_CAPABILITY_PRINCIPAL: &str = "opentopia:unelevated-capability:v1";
const UNELEVATED_CAPABILITY_PRINCIPAL_PREFIX: &str = "opentopia:unelevated-capability:v2:";
const UNELEVATED_CAPABILITY_NAMESPACE: u128 = 0xa678_2ac1_8754_5ef2_99b0_b62a_15c7_c90e;

#[derive(serde::Serialize, serde::Deserialize)]
struct ElevatedRunnerRequestEnvelope {
    protocol_version: u32,
    request: SandboxRequest,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ElevatedRunnerResultEnvelope {
    protocol_version: u32,
    result: ElevatedRunnerResult,
}

fn run_elevated(request: SandboxRequest) -> Result<i32> {
    crate::logging::event(
        "prepare_sandbox",
        "starting elevated dedicated-user backend",
    );
    let credentials = crate::setup::load_credentials().map_err(|error| {
        anyhow::anyhow!("stage=prepare_sandbox elevated backend unavailable: {error:#}")
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
    ensure_persistent_user_permissions(&request, username, user_sid.as_ptr())?;
    let _protected_write_lock = (!request.allowed_protected_roots.is_empty())
        .then(NamedAclMutex::acquire)
        .transpose()?;
    let _protected_write_window =
        ProtectedWriteWindow::open(request.allowed_protected_roots.clone(), user_sid.as_ptr())?;

    let run_root = std::env::temp_dir().join(format!(
        "opentopia-elevated-run-{}",
        Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&run_root).with_context(|| {
        format!(
            "stage=prepare_sandbox create elevated run directory {}",
            run_root.display()
        )
    })?;
    let _cleanup = RunDirectory(run_root.clone());
    update_dacl(
        &run_root,
        user_sid.as_ptr(),
        GRANT_ACCESS,
        true,
        FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | FILE_GENERIC_WRITE,
    )?;

    let request_path = run_root.join("request.json");
    let result_path = run_root.join("result.json");
    let stdout_path = run_root.join("stdout.bin");
    let stderr_path = run_root.join("stderr.bin");
    crate::setup::ensure_parent(&request_path)?;
    std::fs::write(
        &request_path,
        serde_json::to_vec(&ElevatedRunnerRequestEnvelope {
            protocol_version: ELEVATED_RUNNER_PROTOCOL_VERSION,
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
    let cwd_w = wide(request.cwd.as_os_str());
    let environment = current_environment_block(Some(&request.cwd), None, true);
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
    crate::logging::event("spawn", format!("elevated runner user={username}"));
    unsafe { CloseHandle(process.hThread) };
    let broker_timeout = request
        .timeout_ms
        .unwrap_or(30_000)
        .saturating_add(request.termination_timeout_ms)
        .saturating_add(15_000)
        .min(u32::MAX as u64) as u32;
    let waited = unsafe { WaitForSingleObject(process.hProcess, broker_timeout) };
    if waited == WAIT_TIMEOUT {
        unsafe {
            TerminateProcess(process.hProcess, 124);
            WaitForSingleObject(
                process.hProcess,
                request.termination_timeout_ms.min(u32::MAX as u64) as u32,
            );
            CloseHandle(process.hProcess);
        }
        anyhow::bail!(
            "stage=wait elevated runner exceeded the command lifecycle timeout of {broker_timeout}ms"
        )
    }
    if waited == u32::MAX {
        unsafe { CloseHandle(process.hProcess) };
        return Err(last_error("stage=wait elevated runner"));
    }
    unsafe { CloseHandle(process.hProcess) };

    forward_file(&stdout_path, std::io::stdout())?;
    forward_file(&stderr_path, std::io::stderr())?;
    let runner_result: ElevatedRunnerResultEnvelope = serde_json::from_slice(
        &std::fs::read(&result_path).context("stage=collect_output read runner result")?,
    )
    .context("stage=collect_output parse runner result")?;
    if runner_result.protocol_version != ELEVATED_RUNNER_PROTOCOL_VERSION {
        anyhow::bail!(
            "stage=collect_output elevated runner protocol mismatch: expected {} got {}",
            ELEVATED_RUNNER_PROTOCOL_VERSION,
            runner_result.protocol_version
        )
    }
    if let Some(error) = runner_result.result.error {
        anyhow::bail!("{error}")
    }
    Ok(runner_result.result.exit_code)
}

pub(super) fn run_elevated_runner(args: &[String]) -> Result<i32> {
    crate::logging::event("runner", "elevated runner started");
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
            value => anyhow::bail!("unexpected elevated runner argument: {value}"),
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
    let envelope: ElevatedRunnerRequestEnvelope =
        serde_json::from_slice(&std::fs::read(request_path)?)?;
    if envelope.protocol_version != ELEVATED_RUNNER_PROTOCOL_VERSION {
        anyhow::bail!(
            "stage=prepare_sandbox elevated runner protocol mismatch: expected {} got {}",
            ELEVATED_RUNNER_PROTOCOL_VERSION,
            envelope.protocol_version
        )
    }
    let result = match launch_elevated_target(&envelope.request, &stdout_path, &stderr_path) {
        Ok(exit_code) => ElevatedRunnerResult {
            exit_code,
            error: None,
        },
        Err(error) => ElevatedRunnerResult {
            exit_code: 1,
            error: Some(format!("{error:#}")),
        },
    };
    std::fs::write(
        result_path,
        serde_json::to_vec(&ElevatedRunnerResultEnvelope {
            protocol_version: ELEVATED_RUNNER_PROTOCOL_VERSION,
            result,
        })?,
    )?;
    Ok(0)
}

fn launch_elevated_target(
    request: &SandboxRequest,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<i32> {
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
    let cwd = wide(request.cwd.as_os_str());
    let profile_home = current_profile_home()?;
    let environment = current_environment_block(Some(&request.cwd), Some(&profile_home), false);
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
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
        return Err(last_error("stage=spawn CreateProcessW"));
    }
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
                            "stage=wait command timed out after {timeout}ms; elevated process tree terminated"
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
                "stage=terminate elevated process tree did not exit within {}ms after {reason}",
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

fn ensure_persistent_user_permissions(
    request: &SandboxRequest,
    account: &str,
    sid: PSID,
) -> Result<()> {
    let _guard = NamedAclMutex::acquire()?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let mut desired = Vec::new();
    desired.extend(
        request
            .denied_read_paths
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .map(|path| (path, PersistentAclKind::DenyRead)),
    );
    desired.extend(
        request
            .read_roots
            .iter()
            .filter(|path| {
                !request
                    .write_roots
                    .iter()
                    .any(|write_root| path_starts_with(path, write_root))
            })
            .cloned()
            .map(|path| (path, PersistentAclKind::Read)),
    );
    desired.extend(
        request
            .runtime_roots
            .iter()
            .filter(|root| needs_explicit_runtime_acl(root, request))
            .cloned()
            .map(|path| (path, PersistentAclKind::Read)),
    );
    desired.extend(
        request
            .write_roots
            .iter()
            .cloned()
            .map(|path| (path, PersistentAclKind::Write)),
    );
    desired.extend(
        request
            .protected_paths
            .iter()
            .filter(|path| path.exists())
            .cloned()
            .map(|path| (path, PersistentAclKind::DenyWrite)),
    );

    for (path, kind) in desired {
        let entry = PersistentAclEntry {
            account: account.to_string(),
            path: path.clone(),
            kind: kind.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        };
        if ledger.entries.contains(&entry) {
            continue;
        }
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
            PersistentAclKind::DenyWrite => transaction.deny_write(&path, sid, true)?,
        }
        ledger.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry);
    }
    save_acl_ledger(&ledger)?;
    transaction.commit();
    Ok(())
}

fn ensure_persistent_capability_permissions(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<()> {
    let _guard = NamedAclMutex::acquire()?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let legacy_entries = ledger
        .entries
        .iter()
        .filter(|entry| entry.account == LEGACY_UNELEVATED_CAPABILITY_PRINCIPAL)
        .cloned()
        .collect::<Vec<_>>();
    if !legacy_entries.is_empty() {
        let mut legacy_sid = SidBuffer::legacy_opentopia_capability();
        for entry in &legacy_entries {
            update_dacl(&entry.path, legacy_sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
        ledger
            .entries
            .retain(|entry| entry.account != LEGACY_UNELEVATED_CAPABILITY_PRINCIPAL);
    }
    let desired = request
        .write_roots
        .iter()
        .cloned()
        .map(|path| (path, PersistentAclKind::Write))
        .chain(
            request
                .protected_paths
                .iter()
                .filter(|path| path.exists())
                .cloned()
                .map(|path| (path, PersistentAclKind::DenyWrite)),
        );

    for (path, kind) in desired {
        let entry = PersistentAclEntry {
            account: principal.to_string(),
            path: path.clone(),
            kind: kind.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        };
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
        ledger.entries.push(entry);
    }
    save_acl_ledger(&ledger)?;
    transaction.commit();
    Ok(())
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
    let _guard = NamedAclMutex::acquire()?;
    let mut ledger = load_acl_ledger()?;
    let mut revoked = BTreeSet::new();
    for entry in ledger.entries.iter().filter(|entry| {
        path_starts_with(&entry.path, &workspace)
            && revoked.insert((entry.account.clone(), entry.path.clone()))
    }) {
        let mut sid = acl_principal_sid(&entry.account)?;
        update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
    }
    ledger
        .entries
        .retain(|entry| !path_starts_with(&entry.path, &workspace));
    save_acl_ledger(&ledger)?;
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

/// Windows system and installed-program directories already grant normal users
/// execute/read access. Rewriting those
/// machine ACLs is both unnecessary and likely to require elevation. Runtime
/// roots outside that baseline receive an explicit grant only when they are not
/// already covered by the request's content roots.
fn needs_explicit_runtime_acl(root: &Path, request: &SandboxRequest) -> bool {
    let covered = request
        .read_roots
        .iter()
        .chain(request.write_roots.iter())
        .any(|candidate| path_starts_with(root, candidate));
    !covered && !platform_runtime_roots().any(|candidate| path_starts_with(root, &candidate))
}

fn platform_runtime_roots() -> impl Iterator<Item = std::path::PathBuf> {
    [
        std::env::var_os("SystemRoot").map(std::path::PathBuf::from),
        std::env::var_os("ProgramFiles").map(std::path::PathBuf::from),
        std::env::var_os("ProgramFiles(x86)").map(std::path::PathBuf::from),
        std::env::var_os("ProgramData").map(std::path::PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .map(|path| path.canonicalize().unwrap_or(path))
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

struct ProtectedWriteWindow {
    paths: Vec<std::path::PathBuf>,
    sid: PSID,
}

impl ProtectedWriteWindow {
    fn open(paths: Vec<std::path::PathBuf>, sid: PSID) -> Result<Self> {
        for path in &paths {
            update_dacl(path, sid, REVOKE_ACCESS, true, 0)?;
        }
        Ok(Self { paths, sid })
    }
}

impl Drop for ProtectedWriteWindow {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = update_dacl(
                path,
                self.sid,
                DENY_ACCESS,
                true,
                WRITE_RESTRICTION_PERMISSIONS,
            );
        }
    }
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
    if principal == LEGACY_UNELEVATED_CAPABILITY_PRINCIPAL {
        Ok(SidBuffer::legacy_opentopia_capability())
    } else if let Some(value) = principal.strip_prefix(UNELEVATED_CAPABILITY_PRINCIPAL_PREFIX) {
        let id = Uuid::parse_str(value).context("parse scoped capability principal")?;
        Ok(SidBuffer::opentopia_capability(id))
    } else {
        account_sid(principal)
    }
}

fn capability_principal(request: &SandboxRequest) -> String {
    let mut roots = request
        .write_roots
        .iter()
        .map(|path| normalized_capability_path(path))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        roots.push(normalized_capability_path(&request.cwd));
    }
    roots.sort_unstable();
    roots.dedup();
    let scope = roots.join("\0");
    let namespace = Uuid::from_u128(UNELEVATED_CAPABILITY_NAMESPACE);
    let id = Uuid::new_v5(&namespace, scope.as_bytes());
    format!("{UNELEVATED_CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple())
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

fn current_profile_home() -> Result<std::path::PathBuf> {
    let mut token = ptr::null_mut();
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(last_error(
            "stage=prepare_sandbox OpenProcessToken(profile)",
        ));
    }
    let mut size = 0;
    unsafe { GetUserProfileDirectoryW(token, ptr::null_mut(), &mut size) };
    if size == 0 {
        unsafe { CloseHandle(token) };
        return Err(last_error(
            "stage=prepare_sandbox GetUserProfileDirectoryW(size)",
        ));
    }
    let mut value = vec![0_u16; size as usize];
    let queried = unsafe { GetUserProfileDirectoryW(token, value.as_mut_ptr(), &mut size) };
    unsafe { CloseHandle(token) };
    if queried == 0 {
        return Err(last_error("stage=prepare_sandbox GetUserProfileDirectoryW"));
    }
    if value.last() == Some(&0) {
        value.pop();
    }
    Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
        &value,
    )))
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
        let mut restricting_sids = [unsafe { std::mem::zeroed::<SID_AND_ATTRIBUTES>() }; 3];
        restricting_sids[0].Sid = capability_sid;
        restricting_sids[1] = logon_sid.as_sid_and_attributes();
        restricting_sids[2] = everyone_sid.as_sid_and_attributes();
        let mut restricted = ptr::null_mut();
        let created = unsafe {
            CreateRestrictedToken(
                base_token,
                DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED,
                0,
                ptr::null(),
                0,
                ptr::null(),
                restricting_sids.len() as u32,
                restricting_sids.as_ptr(),
                &mut restricted,
            )
        };
        unsafe { CloseHandle(base_token) };
        if created == 0 || restricted.is_null() {
            return Err(last_error("CreateRestrictedToken"));
        }
        if let Err(error) = set_default_dacl(
            restricted,
            &[capability_sid, logon_sid.as_ptr(), everyone_sid.as_ptr()],
        ) {
            unsafe { CloseHandle(restricted) };
            return Err(error);
        }
        Ok(Self { handle: restricted })
    }
}

impl Drop for RestrictedToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
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

    fn as_sid_and_attributes(&mut self) -> SID_AND_ATTRIBUTES {
        SID_AND_ATTRIBUTES {
            Sid: self.as_ptr(),
            Attributes: 0,
        }
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

#[derive(Default)]
struct AclTransaction {
    changes: Vec<AclChange>,
}

struct AclChange {
    path: std::path::PathBuf,
    sid: PSID,
}

impl AclTransaction {
    fn grant(&mut self, path: &Path, sid: PSID, inherit: bool, permissions: u32) -> Result<()> {
        update_dacl(path, sid, GRANT_ACCESS, inherit, permissions)?;
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid,
        });
        Ok(())
    }

    fn deny_write(&mut self, path: &Path, sid: PSID, inherit: bool) -> Result<()> {
        self.deny(path, sid, inherit, WRITE_RESTRICTION_PERMISSIONS)
    }

    fn deny(&mut self, path: &Path, sid: PSID, inherit: bool, permissions: u32) -> Result<()> {
        update_dacl(path, sid, DENY_ACCESS, inherit, permissions)?;
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid,
        });
        Ok(())
    }

    fn commit(mut self) {
        self.changes.clear();
    }
}

impl Drop for AclTransaction {
    fn drop(&mut self) {
        for change in self.changes.iter().rev() {
            let _ = update_dacl(&change.path, change.sid, REVOKE_ACCESS, false, 0);
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
    let _acl_guard = NamedAclMutex::acquire()?;
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
    fn acquire() -> Result<Self> {
        let name = wide(OsStr::new("Local\\OpenTopiaSandboxAcl"));
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return Err(last_error("stage=apply_acl CreateMutexW"));
        }
        let waited = unsafe { WaitForSingleObject(handle, 30_000) };
        if waited == WAIT_TIMEOUT {
            unsafe { CloseHandle(handle) };
            anyhow::bail!("stage=apply_acl timed out waiting for the ACL transaction lock")
        }
        if waited == u32::MAX {
            unsafe { CloseHandle(handle) };
            return Err(last_error("stage=apply_acl WaitForSingleObject"));
        }
        Ok(Self(handle))
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
    fn elevated_runner_envelopes_are_explicitly_versioned() {
        let request = SandboxRequest {
            interactive: false,
            cwd: Path::new(r"C:\workspace").to_path_buf(),
            read_roots: Vec::new(),
            runtime_roots: Vec::new(),
            write_roots: Vec::new(),
            protected_paths: Vec::new(),
            denied_read_paths: Vec::new(),
            allowed_protected_roots: Vec::new(),
            network: NetworkMode::Deny,
            timeout_ms: Some(1_000),
            termination_timeout_ms: 500,
            max_memory_bytes: None,
            max_cpu_time_ms: None,
            max_output_bytes: None,
            backend: BackendMode::Elevated,
            command: vec!["cmd.exe".to_string()],
        };
        let encoded = serde_json::to_vec(&ElevatedRunnerRequestEnvelope {
            protocol_version: ELEVATED_RUNNER_PROTOCOL_VERSION,
            request,
        })
        .expect("serialize request envelope");
        let decoded: ElevatedRunnerRequestEnvelope =
            serde_json::from_slice(&encoded).expect("deserialize request envelope");
        assert_eq!(decoded.protocol_version, ELEVATED_RUNNER_PROTOCOL_VERSION);
    }

    #[test]
    fn unelevated_capability_principal_is_stable_and_scope_specific() {
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

    fn capability_request(write_roots: &[&str]) -> SandboxRequest {
        SandboxRequest {
            interactive: false,
            cwd: Path::new(r"C:\workspace").to_path_buf(),
            read_roots: Vec::new(),
            runtime_roots: Vec::new(),
            write_roots: write_roots.iter().map(std::path::PathBuf::from).collect(),
            protected_paths: Vec::new(),
            denied_read_paths: Vec::new(),
            allowed_protected_roots: Vec::new(),
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
