use crate::dpapi;
use anyhow::{Context, Result};
use opentopia_sandbox_protocol::{SandboxSetupComponents, SandboxSetupState, SandboxSetupStatus};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND,
    ERROR_PATH_NOT_FOUND, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NERR_Success, NERR_UserExists, NERR_UserNotFound, NetApiBufferFree, NetUserAdd, NetUserDel,
    NetUserGetInfo, NetUserSetInfo, UF_DONT_EXPIRE_PASSWD, UF_SCRIPT, USER_INFO_1, USER_INFO_1003,
    USER_INFO_1007, USER_INFO_4, USER_PRIV_USER,
};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{
    AllocateAndInitializeSid, CheckTokenMembership, CopySid, FreeSid, GetLengthSid, LogonUserW,
    LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, PSID, SECURITY_NT_AUTHORITY,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{
    DeleteProfileW, ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub(crate) const OFFLINE_USERNAME: &str = "OpenTopiaSbOffline";
pub(crate) const ONLINE_USERNAME: &str = "OpenTopiaSbOnline";
const ACCOUNT_COMMENT: &str = "Managed by OpenTopia dedicated-user sandbox";
const LOCAL_ACCOUNT_NAME_MAX_UTF16: usize = 20;
const LIFECYCLE_RESULT_VERSION: u32 = 1;
const LIFECYCLE_RESULT_DIR: &str = "lifecycle-results";

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivilegedOperationResult {
    version: u32,
    action: String,
    success: bool,
    error: Option<String>,
}

#[derive(Debug)]
struct PrivilegedInvocation {
    already_elevated: bool,
    result_nonce: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SandboxCredentials {
    pub offline_username: String,
    pub offline_password: String,
    pub online_username: String,
    pub online_password: String,
    #[serde(default)]
    pub offline_sid: Vec<u8>,
    #[serde(default)]
    pub online_sid: Vec<u8>,
}

pub(crate) fn state_dir() -> PathBuf {
    std::env::var_os("OPENTOPIA_SANDBOX_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|root| root.join("OpenTopia").join("sandbox"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("opentopia-sandbox-state"))
}

fn credentials_path() -> PathBuf {
    state_dir().join("credentials.dpapi")
}

pub(crate) fn credentials_present() -> bool {
    credentials_path().is_file()
}

pub(crate) fn run_setup(args: &[String]) -> Result<i32> {
    if args == ["--status", "--json"] {
        println!("{}", serde_json::to_string(&provisioning_status())?);
        return Ok(0);
    }
    if args == ["--status"] {
        let status = provisioning_status();
        println!(
            "OpenTopia dedicated-user sandbox: {} state={} issues={}",
            setup_state_label(status.state),
            state_dir().display(),
            status.issues.len(),
        );
        return Ok(if status.is_ready() { 0 } else { 1 });
    }
    let invocation =
        parse_privileged_invocation(args, "usage: opentopia-sandbox setup [--status [--json]]")?;
    let administrator = match is_administrator() {
        Ok(value) => value,
        Err(error) => {
            return finish_privileged_operation(
                "setup",
                invocation.result_nonce.as_deref(),
                Err(error),
            )
        }
    };
    if !administrator {
        if invocation.already_elevated {
            return finish_privileged_operation(
                "setup",
                invocation.result_nonce.as_deref(),
                Err(anyhow::anyhow!(
                    "sandbox setup requires administrator privileges"
                )),
            );
        }
        return elevate_operation("setup", true);
    }
    finish_privileged_operation("setup", invocation.result_nonce.as_deref(), perform_setup())
}

fn perform_setup() -> Result<i32> {
    let stored_credentials = reusable_credentials();
    if stored_credentials.is_none() {
        ensure_reserved_identity_available(OFFLINE_USERNAME)?;
        ensure_reserved_identity_available(ONLINE_USERNAME)?;
    }
    let mut credentials = stored_credentials.unwrap_or_else(new_credentials);
    // Publish recoverable intent before making machine changes. A failed
    // account/WFP step is then reported as degraded and can be repaired with
    // the same identities instead of producing unknown partial state.
    save_credentials(&credentials)?;
    ensure_user(&credentials.offline_username, &credentials.offline_password)?;
    ensure_user(&credentials.online_username, &credentials.online_password)?;
    credentials.offline_sid = account_sid_bytes(&credentials.offline_username)?;
    credentials.online_sid = account_sid_bytes(&credentials.online_username)?;
    save_credentials(&credentials)?;
    crate::wfp::install_offline_filters(&credentials.offline_username)
        .context("install offline-user WFP filters")?;
    crate::windows::prepare_setup_canaries()
        .context("prepare dedicated-user execution canaries")?;
    let status = provisioning_status();
    anyhow::ensure!(
        status.is_ready(),
        "dedicated-user sandbox repair did not reach ready state: {}",
        status.issues.join("; ")
    );
    println!(
        "OpenTopia dedicated-user sandbox ready: offline={} online={} state={}",
        credentials.offline_username,
        credentials.online_username,
        state_dir().display()
    );
    Ok(0)
}

pub(crate) fn run_teardown(args: &[String]) -> Result<i32> {
    let invocation = parse_privileged_invocation(args, "usage: opentopia-sandbox teardown")?;
    let administrator = match is_administrator() {
        Ok(value) => value,
        Err(error) => {
            return finish_privileged_operation(
                "teardown",
                invocation.result_nonce.as_deref(),
                Err(error),
            )
        }
    };
    if !administrator {
        if invocation.already_elevated {
            return finish_privileged_operation(
                "teardown",
                invocation.result_nonce.as_deref(),
                Err(anyhow::anyhow!(
                    "sandbox removal requires administrator privileges"
                )),
            );
        }
        return elevate_operation("teardown", false);
    }
    finish_privileged_operation(
        "teardown",
        invocation.result_nonce.as_deref(),
        perform_teardown(),
    )
}

fn perform_teardown() -> Result<i32> {
    let credentials = reusable_credentials();
    ensure_user_removable(
        OFFLINE_USERNAME,
        credentials
            .as_ref()
            .map(|value| value.offline_password.as_str()),
    )?;
    ensure_user_removable(
        ONLINE_USERNAME,
        credentials
            .as_ref()
            .map(|value| value.online_password.as_str()),
    )?;
    crate::windows::revoke_dedicated_user_permissions(&[OFFLINE_USERNAME, ONLINE_USERNAME])
        .context("revoke dedicated-user filesystem permissions")?;
    crate::wfp::remove_offline_filters().context("remove offline-user WFP filters")?;
    delete_user(
        OFFLINE_USERNAME,
        credentials
            .as_ref()
            .map(|value| value.offline_password.as_str()),
    )?;
    delete_user(
        ONLINE_USERNAME,
        credentials
            .as_ref()
            .map(|value| value.online_password.as_str()),
    )?;
    match std::fs::remove_file(credentials_path()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("remove sandbox credentials"),
    }
    println!(
        "OpenTopia dedicated-user sandbox removed: state={}",
        state_dir().display()
    );
    Ok(0)
}

fn elevate_operation(action: &str, prepare_state_dir: bool) -> Result<i32> {
    if prepare_state_dir {
        // Machine-scope DPAPI keeps over-the-shoulder UAC compatible. Creating
        // the directory first preserves the interactive caller's file ACL.
        std::fs::create_dir_all(state_dir())
            .context("create sandbox state directory before UAC")?;
    }
    let result_nonce = Uuid::new_v4().simple().to_string();
    let result_path = lifecycle_result_path(&result_nonce)?;
    let result_dir = result_path
        .parent()
        .context("resolve privileged operation result directory")?;
    std::fs::create_dir_all(result_dir)
        .context("create privileged operation result directory before UAC")?;
    let parameters = format!("{action} --already-elevated --result-nonce {result_nonce}");
    let executable = std::env::current_exe().context("resolve sandbox setup executable")?;
    let executable_w = wide(executable.as_os_str());
    let verb_w = wide("runas");
    let parameters_w = wide(parameters);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb_w.as_ptr();
    info.lpFile = executable_w.as_ptr();
    info.lpParameters = parameters_w.as_ptr();
    info.nShow = SW_HIDE;
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if launched == 0 || info.hProcess.is_null() || info.hProcess == INVALID_HANDLE_VALUE {
        let error = unsafe { GetLastError() };
        cleanup_lifecycle_result(&result_path);
        if error == ERROR_CANCELLED {
            anyhow::bail!("administrator approval was declined")
        }
        anyhow::bail!("failed to request administrator approval: Windows error {error}")
    }
    unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
    let mut exit_code = 1;
    let read = unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) };
    unsafe { CloseHandle(info.hProcess) };
    if read == 0 {
        cleanup_lifecycle_result(&result_path);
        anyhow::bail!("failed to read privileged sandbox configuration result")
    }
    let result = std::fs::read(&result_path)
        .with_context(|| {
            format!(
                "privileged sandbox {action} exited with code {exit_code} without returning diagnostics"
            )
        })
        .and_then(|bytes| {
            serde_json::from_slice::<PrivilegedOperationResult>(&bytes)
                .context("parse privileged sandbox operation result")
        });
    cleanup_lifecycle_result(&result_path);
    let result = result?;
    anyhow::ensure!(
        result.version == LIFECYCLE_RESULT_VERSION,
        "unsupported privileged sandbox result version {}",
        result.version
    );
    anyhow::ensure!(
        result.action == action,
        "privileged sandbox result action mismatch: expected {action}, received {}",
        result.action
    );
    if !result.success {
        anyhow::bail!(
            "privileged sandbox {action} failed: {}",
            result
                .error
                .unwrap_or_else(|| format!("process exited with code {exit_code}"))
        )
    }
    anyhow::ensure!(
        exit_code == 0,
        "privileged sandbox {action} reported success but exited with code {exit_code}"
    );
    Ok(0)
}

fn parse_privileged_invocation(args: &[String], usage: &str) -> Result<PrivilegedInvocation> {
    match args {
        [] => Ok(PrivilegedInvocation {
            already_elevated: false,
            result_nonce: None,
        }),
        [flag] if flag == "--already-elevated" => Ok(PrivilegedInvocation {
            already_elevated: true,
            result_nonce: None,
        }),
        [elevated, result_flag, nonce]
            if elevated == "--already-elevated" && result_flag == "--result-nonce" =>
        {
            lifecycle_result_path(nonce)?;
            Ok(PrivilegedInvocation {
                already_elevated: true,
                result_nonce: Some(nonce.clone()),
            })
        }
        _ => anyhow::bail!("{usage}"),
    }
}

fn lifecycle_result_path(nonce: &str) -> Result<PathBuf> {
    anyhow::ensure!(
        nonce.len() == 32 && nonce.bytes().all(|value| value.is_ascii_hexdigit()),
        "invalid privileged operation result nonce"
    );
    Ok(state_dir()
        .join(LIFECYCLE_RESULT_DIR)
        .join(format!("{nonce}.json")))
}

fn cleanup_lifecycle_result(result_path: &Path) {
    let _ = std::fs::remove_file(result_path);
    if let Some(parent) = result_path.parent() {
        let _ = std::fs::remove_dir(parent);
    }
}

fn finish_privileged_operation(
    action: &str,
    result_nonce: Option<&str>,
    operation: Result<i32>,
) -> Result<i32> {
    let Some(result_nonce) = result_nonce else {
        return operation;
    };
    let result_path = lifecycle_result_path(result_nonce)?;
    let result = PrivilegedOperationResult {
        version: LIFECYCLE_RESULT_VERSION,
        action: action.to_string(),
        success: operation.is_ok(),
        error: operation.as_ref().err().map(|error| format!("{error:#}")),
    };
    let serialized = serde_json::to_vec(&result)?;
    if let Err(write_error) = std::fs::write(&result_path, serialized) {
        return match operation {
            Ok(_) => Err(write_error).with_context(|| {
                format!(
                    "write privileged sandbox {action} result {}",
                    result_path.display()
                )
            }),
            Err(operation_error) => anyhow::bail!(
                "{operation_error:#}; additionally failed to write privileged result {}: {write_error}",
                result_path.display()
            ),
        };
    }
    operation
}

pub(crate) fn provisioning_status() -> SandboxSetupStatus {
    let credential_file_present = credentials_path().is_file();
    let mut issues = Vec::new();
    let credentials = if credential_file_present {
        match load_credentials() {
            Ok(credentials)
                if credentials.offline_username == OFFLINE_USERNAME
                    && credentials.online_username == ONLINE_USERNAME =>
            {
                Some(credentials)
            }
            Ok(_) => {
                issues.push("stored sandbox identities do not match this OpenTopia version".into());
                None
            }
            Err(error) => {
                issues.push(format!("sandbox credentials are unreadable: {error:#}"));
                None
            }
        }
    } else {
        None
    };

    let offline_account = checked_account_exists(OFFLINE_USERNAME, &mut issues);
    let online_account = checked_account_exists(ONLINE_USERNAME, &mut issues);
    let network_policy = match crate::wfp::offline_filters_installed() {
        Ok(installed) => installed,
        Err(error) => {
            issues.push(format!(
                "offline network policy health check failed: {error:#}"
            ));
            false
        }
    };
    let filesystem_permissions = match crate::windows::has_dedicated_user_permissions(&[
        OFFLINE_USERNAME,
        ONLINE_USERNAME,
    ]) {
        Ok(present) => present,
        Err(error) => {
            issues.push(format!(
                "dedicated-user filesystem permission health check failed: {error:#}"
            ));
            false
        }
    };
    let any_artifact = credential_file_present
        || filesystem_permissions
        || offline_account
        || online_account
        || network_policy;

    let credentials_complete = credentials
        .as_ref()
        .is_some_and(|value| !value.offline_sid.is_empty() && !value.online_sid.is_empty());
    let components = SandboxSetupComponents {
        credentials: credentials_complete,
        offline_identity: credentials
            .as_ref()
            .filter(|_| offline_account)
            .is_some_and(|value| {
                checked_identity(
                    &value.offline_username,
                    &value.offline_password,
                    &value.offline_sid,
                    "offline",
                    &mut issues,
                )
            }),
        online_identity: credentials
            .as_ref()
            .filter(|_| online_account)
            .is_some_and(|value| {
                checked_identity(
                    &value.online_username,
                    &value.online_password,
                    &value.online_sid,
                    "online",
                    &mut issues,
                )
            }),
        offline_network_policy: network_policy,
    };
    if components.credentials
        && components.offline_identity
        && components.online_identity
        && components.offline_network_policy
    {
        issues.extend(crate::windows::verify_setup_canaries());
    }
    if any_artifact {
        if !components.credentials && !credential_file_present {
            issues.push("sandbox credentials are missing".into());
        } else if !components.credentials {
            issues.push("sandbox identity metadata is incomplete".into());
        }
        if !offline_account {
            issues.push("offline sandbox account is missing".into());
        }
        if !online_account {
            issues.push("online sandbox account is missing".into());
        }
        if !network_policy {
            issues.push("offline sandbox network policy is missing".into());
        }
    }
    issues.sort();
    issues.dedup();

    let state = setup_state(any_artifact, &components, &issues);
    SandboxSetupStatus::current(state, state_dir().display().to_string(), components, issues)
}

fn setup_state(
    any_artifact: bool,
    components: &SandboxSetupComponents,
    issues: &[String],
) -> SandboxSetupState {
    if components.credentials
        && components.offline_identity
        && components.online_identity
        && components.offline_network_policy
        && issues.is_empty()
    {
        SandboxSetupState::Ready
    } else if any_artifact || !issues.is_empty() {
        SandboxSetupState::Degraded
    } else {
        SandboxSetupState::NotConfigured
    }
}

fn setup_state_label(state: SandboxSetupState) -> &'static str {
    match state {
        SandboxSetupState::NotConfigured => "not-configured",
        SandboxSetupState::Ready => "ready",
        SandboxSetupState::Degraded => "degraded",
    }
}

fn is_administrator() -> Result<bool> {
    const SECURITY_BUILTIN_DOMAIN_RID: u32 = 0x20;
    const DOMAIN_ALIAS_RID_ADMINS: u32 = 0x220;
    let mut sid: PSID = std::ptr::null_mut();
    let allocated = unsafe {
        AllocateAndInitializeSid(
            &SECURITY_NT_AUTHORITY,
            2,
            SECURITY_BUILTIN_DOMAIN_RID,
            DOMAIN_ALIAS_RID_ADMINS,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut sid,
        )
    };
    if allocated == 0 {
        anyhow::bail!("AllocateAndInitializeSid failed while checking setup privileges")
    }
    let mut is_member = 0;
    let checked = unsafe { CheckTokenMembership(std::ptr::null_mut(), sid, &mut is_member) };
    unsafe { FreeSid(sid) };
    if checked == 0 {
        anyhow::bail!("CheckTokenMembership failed while checking setup privileges")
    }
    Ok(is_member != 0)
}

fn checked_account_exists(username: &str, issues: &mut Vec<String>) -> bool {
    match account_exists(username) {
        Ok(exists) => exists,
        Err(error) => {
            issues.push(format!(
                "failed to inspect sandbox account {username}: {error:#}"
            ));
            false
        }
    }
}

fn account_exists(username: &str) -> Result<bool> {
    let username_w = wide(username);
    let mut buffer = std::ptr::null_mut();
    let status = unsafe { NetUserGetInfo(std::ptr::null(), username_w.as_ptr(), 0, &mut buffer) };
    if !buffer.is_null() {
        unsafe { NetApiBufferFree(buffer.cast()) };
    }
    if status == NERR_Success {
        Ok(true)
    } else if status == NERR_UserNotFound {
        Ok(false)
    } else {
        anyhow::bail!("NetUserGetInfo returned {status}")
    }
}

fn ensure_reserved_identity_available(username: &str) -> Result<()> {
    validate_local_account_name(username)?;
    if account_exists(username)? && !account_is_managed(username)? {
        anyhow::bail!(
            "refusing to reuse existing local account {username} because it is not owned by OpenTopia"
        )
    }
    Ok(())
}

fn account_is_managed(username: &str) -> Result<bool> {
    let username_w = wide(username);
    let mut buffer = std::ptr::null_mut();
    // USER_INFO_1007 is a write-only NetUserSetInfo level. NetUserGetInfo
    // exposes the comment through USER_INFO_1 instead.
    let status = unsafe { NetUserGetInfo(std::ptr::null(), username_w.as_ptr(), 1, &mut buffer) };
    if status == NERR_UserNotFound {
        return Ok(false);
    }
    if status != NERR_Success {
        anyhow::bail!("NetUserGetInfo(comment) returned {status}")
    }
    let managed = if buffer.is_null() {
        false
    } else {
        let info = unsafe { &*(buffer.cast::<USER_INFO_1>()) };
        wide_ptr_string(info.usri1_comment).as_deref() == Some(ACCOUNT_COMMENT)
    };
    if !buffer.is_null() {
        unsafe { NetApiBufferFree(buffer.cast()) };
    }
    Ok(managed)
}

fn wide_ptr_string(value: *const u16) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut len = 0;
    while unsafe { *value.add(len) } != 0 {
        len += 1;
    }
    Some(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(value, len)
    }))
}

fn checked_logon(username: &str, password: &str, identity: &str, issues: &mut Vec<String>) -> bool {
    match validate_logon(username, password) {
        Ok(()) => true,
        Err(error) => {
            issues.push(format!(
                "{identity} sandbox identity cannot log on: {error:#}"
            ));
            false
        }
    }
}

fn checked_identity(
    username: &str,
    password: &str,
    expected_sid: &[u8],
    identity: &str,
    issues: &mut Vec<String>,
) -> bool {
    if expected_sid.is_empty() {
        issues.push(format!("{identity} sandbox identity SID is missing"));
        return false;
    }
    match account_sid_bytes(username) {
        Ok(actual_sid) if actual_sid == expected_sid => {}
        Ok(_) => {
            issues.push(format!("{identity} sandbox identity SID has changed"));
            return false;
        }
        Err(error) => {
            issues.push(format!(
                "failed to inspect {identity} sandbox identity SID: {error:#}"
            ));
            return false;
        }
    }
    checked_logon(username, password, identity, issues)
}

fn account_sid_bytes(username: &str) -> Result<Vec<u8>> {
    let username_w = wide(username);
    let mut buffer = std::ptr::null_mut();
    let status = unsafe { NetUserGetInfo(std::ptr::null(), username_w.as_ptr(), 4, &mut buffer) };
    if status != NERR_Success {
        if !buffer.is_null() {
            unsafe { NetApiBufferFree(buffer.cast()) };
        }
        anyhow::bail!("NetUserGetInfo(SID) returned {status}")
    }
    anyhow::ensure!(!buffer.is_null(), "NetUserGetInfo(SID) returned no data");
    let sid = unsafe { (*(buffer.cast::<USER_INFO_4>())).usri4_user_sid };
    let size = unsafe { GetLengthSid(sid) };
    if size == 0 {
        unsafe { NetApiBufferFree(buffer.cast()) };
        anyhow::bail!("GetLengthSid returned Windows error {}", unsafe {
            GetLastError()
        })
    }
    let mut bytes = vec![0; size as usize];
    let copied = unsafe { CopySid(size, bytes.as_mut_ptr().cast(), sid) };
    unsafe { NetApiBufferFree(buffer.cast()) };
    if copied == 0 {
        anyhow::bail!("CopySid returned Windows error {}", unsafe {
            GetLastError()
        })
    }
    Ok(bytes)
}

fn validate_logon(username: &str, password: &str) -> Result<()> {
    let username_w = wide(username);
    let domain_w = wide(".");
    let password_w = wide(password);
    let mut token = std::ptr::null_mut();
    let logged_on = unsafe {
        LogonUserW(
            username_w.as_ptr(),
            domain_w.as_ptr(),
            password_w.as_ptr(),
            LOGON32_LOGON_INTERACTIVE,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };
    if logged_on == 0 {
        anyhow::bail!("LogonUserW returned Windows error {}", unsafe {
            GetLastError()
        })
    }
    unsafe { CloseHandle(token) };
    Ok(())
}

fn reusable_credentials() -> Option<SandboxCredentials> {
    load_credentials().ok().filter(|credentials| {
        credentials.offline_username == OFFLINE_USERNAME
            && credentials.online_username == ONLINE_USERNAME
    })
}

fn new_credentials() -> SandboxCredentials {
    SandboxCredentials {
        offline_username: OFFLINE_USERNAME.to_string(),
        offline_password: random_password(),
        online_username: ONLINE_USERNAME.to_string(),
        online_password: random_password(),
        offline_sid: Vec::new(),
        online_sid: Vec::new(),
    }
}

pub(crate) fn load_credentials() -> Result<SandboxCredentials> {
    let encrypted = std::fs::read(credentials_path())
        .context("dedicated-user sandbox setup is incomplete; run `opentopia-sandbox setup`")?;
    let bytes = dpapi::unprotect(&encrypted).context("decrypt sandbox credentials")?;
    serde_json::from_slice(&bytes).context("parse sandbox credentials")
}

fn save_credentials(credentials: &SandboxCredentials) -> Result<()> {
    let root = state_dir();
    std::fs::create_dir_all(&root)
        .with_context(|| format!("create sandbox state directory {}", root.display()))?;
    let serialized = serde_json::to_vec(credentials)?;
    let encrypted = dpapi::protect(&serialized)?;
    let path = credentials_path();
    std::fs::write(&path, encrypted)
        .with_context(|| format!("write sandbox credentials {}", path.display()))
}

fn random_password() -> String {
    let a = Uuid::new_v4().simple().to_string();
    let b = Uuid::new_v4().simple().to_string();
    format!("Ot!{a}{b}9z")
}

fn ensure_user(username: &str, password: &str) -> Result<()> {
    validate_local_account_name(username)?;
    let username_w = wide(username);
    let password_w = wide(password);
    let comment_w = wide(ACCOUNT_COMMENT);
    let info = USER_INFO_1 {
        usri1_name: username_w.as_ptr() as *mut u16,
        usri1_password: password_w.as_ptr() as *mut u16,
        usri1_password_age: 0,
        usri1_priv: USER_PRIV_USER,
        usri1_home_dir: std::ptr::null_mut(),
        usri1_comment: comment_w.as_ptr() as *mut u16,
        usri1_flags: UF_SCRIPT | UF_DONT_EXPIRE_PASSWD,
        usri1_script_path: std::ptr::null_mut(),
    };
    let status = unsafe {
        NetUserAdd(
            std::ptr::null(),
            1,
            &info as *const _ as *mut u8,
            std::ptr::null_mut(),
        )
    };
    if status == NERR_Success {
        return Ok(());
    }
    if status != NERR_UserExists {
        anyhow::bail!(
            "NetUserAdd failed for {username}: status={status}; administrator approval is required"
        )
    }
    if !account_is_managed(username)? && validate_logon(username, password).is_err() {
        anyhow::bail!(
            "refusing to modify existing local account {username} because OpenTopia ownership could not be verified"
        )
    }
    let password_info = USER_INFO_1003 {
        usri1003_password: password_w.as_ptr() as *mut u16,
    };
    let update = unsafe {
        NetUserSetInfo(
            std::ptr::null(),
            username_w.as_ptr(),
            1003,
            &password_info as *const _ as *mut u8,
            std::ptr::null_mut(),
        )
    };
    if update != NERR_Success {
        anyhow::bail!(
            "NetUserAdd/NetUserSetInfo failed for {username}: create={status} update={update}; administrator approval is required"
        )
    }
    let comment_info = USER_INFO_1007 {
        usri1007_comment: comment_w.as_ptr() as *mut u16,
    };
    let comment_update = unsafe {
        NetUserSetInfo(
            std::ptr::null(),
            username_w.as_ptr(),
            1007,
            &comment_info as *const _ as *mut u8,
            std::ptr::null_mut(),
        )
    };
    if comment_update != NERR_Success {
        anyhow::bail!("NetUserSetInfo(comment) failed for {username}: status={comment_update}")
    }
    Ok(())
}

fn validate_local_account_name(username: &str) -> Result<()> {
    let utf16_length = username.encode_utf16().count();
    anyhow::ensure!(!username.is_empty(), "local sandbox account name is empty");
    anyhow::ensure!(
        utf16_length <= LOCAL_ACCOUNT_NAME_MAX_UTF16,
        "local sandbox account name {username} is {utf16_length} UTF-16 code units; Windows permits at most {LOCAL_ACCOUNT_NAME_MAX_UTF16}"
    );
    Ok(())
}

fn ensure_user_removable(username: &str, expected_password: Option<&str>) -> Result<()> {
    if !account_exists(username)? {
        return Ok(());
    }
    let owned = account_is_managed(username)?
        || expected_password.is_some_and(|password| validate_logon(username, password).is_ok());
    anyhow::ensure!(
        owned,
        "refusing to delete existing local account {username} because OpenTopia ownership could not be verified"
    );
    Ok(())
}

fn delete_user(username: &str, expected_password: Option<&str>) -> Result<()> {
    ensure_user_removable(username, expected_password)?;
    if !account_exists(username)? {
        return Ok(());
    }
    let sid = account_sid_bytes(username)?;
    delete_profile(&sid)?;
    let username_w = wide(username);
    let status = unsafe { NetUserDel(std::ptr::null(), username_w.as_ptr()) };
    if status != NERR_Success && status != NERR_UserNotFound {
        anyhow::bail!(
            "NetUserDel failed for {username}: status={status}; administrator approval is required"
        )
    }
    Ok(())
}

fn delete_profile(sid: &[u8]) -> Result<()> {
    if sid.is_empty() {
        return Ok(());
    }
    let mut sid = sid.to_vec();
    let mut sid_string = std::ptr::null_mut();
    let converted = unsafe { ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &mut sid_string) };
    if converted == 0 {
        anyhow::bail!("ConvertSidToStringSidW returned Windows error {}", unsafe {
            GetLastError()
        })
    }
    let deleted = unsafe { DeleteProfileW(sid_string, std::ptr::null(), std::ptr::null()) };
    let error = if deleted == 0 {
        Some(unsafe { GetLastError() })
    } else {
        None
    };
    unsafe { LocalFree(sid_string as HLOCAL) };
    match error {
        None | Some(ERROR_FILE_NOT_FOUND) | Some(ERROR_PATH_NOT_FOUND) => Ok(()),
        Some(value) => anyhow::bail!("DeleteProfileW returned Windows error {value}"),
    }
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

pub(crate) fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_account_names_respect_windows_sam_limit() {
        assert!(validate_local_account_name(OFFLINE_USERNAME).is_ok());
        assert!(validate_local_account_name(ONLINE_USERNAME).is_ok());
        assert!(validate_local_account_name("OpenTopiaSandboxOffline").is_err());
    }

    #[test]
    fn privileged_result_nonce_is_fixed_width_hex() {
        assert!(lifecycle_result_path("00112233445566778899aabbccddeeff").is_ok());
        assert!(lifecycle_result_path("../outside").is_err());
        assert!(lifecycle_result_path("0011").is_err());
    }

    #[test]
    fn provisioning_state_distinguishes_absent_ready_and_partial_installations() {
        assert_eq!(
            setup_state(false, &SandboxSetupComponents::default(), &[]),
            SandboxSetupState::NotConfigured
        );
        assert_eq!(
            setup_state(
                true,
                &SandboxSetupComponents {
                    credentials: true,
                    offline_identity: true,
                    online_identity: true,
                    offline_network_policy: true,
                },
                &[],
            ),
            SandboxSetupState::Ready
        );
        assert_eq!(
            setup_state(true, &SandboxSetupComponents::default(), &[]),
            SandboxSetupState::Degraded
        );
    }
}
