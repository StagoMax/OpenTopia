use super::SandboxRequest;
use crate::process_env::current_environment_block;
use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_JOB_TIME, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

pub(super) fn launch(request: &SandboxRequest, restricted_token: HANDLE) -> Result<u32> {
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

pub(super) fn create_job(request: &SandboxRequest) -> Result<HANDLE> {
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

pub(super) struct AttributeList {
    storage: Vec<u8>,
}

impl AttributeList {
    pub(super) fn new(count: u32) -> Result<Self> {
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

    pub(super) fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }

    pub(super) fn update(
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

pub(super) fn argv_to_command_line(argv: &[String]) -> String {
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

pub(super) fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value
        .as_ref()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub(super) fn native_path(path: &Path) -> std::path::PathBuf {
    let display = path.as_os_str().to_string_lossy();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(native) = display.strip_prefix(r"\\?\") {
        return std::path::PathBuf::from(native);
    }
    path.to_path_buf()
}

pub(super) fn last_error(operation: &str) -> anyhow::Error {
    anyhow::anyhow!("{operation} failed: {}", unsafe { GetLastError() })
}

#[cfg(test)]
mod tests {
    use super::native_path;
    use std::path::Path;

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
}
