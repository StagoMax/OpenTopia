use crate::dpapi;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NERR_Success, NetUserAdd, NetUserSetInfo, UF_DONT_EXPIRE_PASSWD, UF_SCRIPT, USER_INFO_1,
    USER_INFO_1003, USER_PRIV_USER,
};
use windows_sys::Win32::Security::{
    AllocateAndInitializeSid, CheckTokenMembership, FreeSid, PSID, SECURITY_NT_AUTHORITY,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

pub(crate) const OFFLINE_USERNAME: &str = "OpenTopiaSandboxOffline";
pub(crate) const ONLINE_USERNAME: &str = "OpenTopiaSandboxOnline";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SandboxCredentials {
    pub offline_username: String,
    pub offline_password: String,
    pub online_username: String,
    pub online_password: String,
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

pub(crate) fn is_complete() -> bool {
    credentials_path().is_file()
}

pub(crate) fn run_setup(args: &[String]) -> Result<i32> {
    if args == ["--status"] {
        println!(
            "OpenTopia elevated sandbox setup: {} state={}",
            if is_complete() {
                "ready"
            } else {
                "not-configured"
            },
            state_dir().display()
        );
        return Ok(if is_complete() { 0 } else { 1 });
    }
    if !args.is_empty() && args != ["--already-elevated"] {
        anyhow::bail!("usage: opentopia-sandbox setup [--status]")
    }
    let already_elevated = args.iter().any(|arg| arg == "--already-elevated");
    if !is_administrator()? {
        if already_elevated {
            anyhow::bail!("sandbox setup requires administrator privileges")
        }
        return elevate_setup();
    }
    let credentials = SandboxCredentials {
        offline_username: OFFLINE_USERNAME.to_string(),
        offline_password: random_password(),
        online_username: ONLINE_USERNAME.to_string(),
        online_password: random_password(),
    };
    ensure_user(&credentials.offline_username, &credentials.offline_password)?;
    ensure_user(&credentials.online_username, &credentials.online_password)?;
    crate::wfp::install_offline_filters(&credentials.offline_username)
        .context("install offline-user WFP filters")?;
    save_credentials(&credentials)?;
    println!(
        "OpenTopia elevated sandbox setup complete: offline={} online={} state={}",
        credentials.offline_username,
        credentials.online_username,
        state_dir().display()
    );
    Ok(0)
}

fn elevate_setup() -> Result<i32> {
    // Create the state directory as the interactive caller so files created by
    // an over-the-shoulder administrator inherit an ACL that the caller can
    // still read after UAC setup completes.
    std::fs::create_dir_all(state_dir()).context("create sandbox state directory before UAC")?;
    let executable = std::env::current_exe().context("resolve sandbox setup executable")?;
    let executable_w = wide(executable.as_os_str());
    let verb_w = wide("runas");
    let parameters_w = wide("setup --already-elevated");
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb_w.as_ptr();
    info.lpFile = executable_w.as_ptr();
    info.lpParameters = parameters_w.as_ptr();
    info.nShow = SW_HIDE;
    let launched = unsafe { ShellExecuteExW(&mut info) };
    if launched == 0 || info.hProcess.is_null() || info.hProcess == INVALID_HANDLE_VALUE {
        anyhow::bail!("failed to request administrator approval for sandbox setup")
    }
    unsafe { WaitForSingleObject(info.hProcess, INFINITE) };
    let mut exit_code = 1;
    let read = unsafe { GetExitCodeProcess(info.hProcess, &mut exit_code) };
    unsafe { CloseHandle(info.hProcess) };
    if read == 0 {
        anyhow::bail!("failed to read elevated sandbox setup result")
    }
    Ok(exit_code as i32)
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

pub(crate) fn load_credentials() -> Result<SandboxCredentials> {
    let encrypted = std::fs::read(credentials_path())
        .context("elevated sandbox setup is incomplete; run `opentopia-sandbox setup`")?;
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
    let username_w = wide(username);
    let password_w = wide(password);
    let info = USER_INFO_1 {
        usri1_name: username_w.as_ptr() as *mut u16,
        usri1_password: password_w.as_ptr() as *mut u16,
        usri1_password_age: 0,
        usri1_priv: USER_PRIV_USER,
        usri1_home_dir: std::ptr::null_mut(),
        usri1_comment: std::ptr::null_mut(),
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
    Ok(())
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
