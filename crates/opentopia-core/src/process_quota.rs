//! Process-tree cleanup and resource quotas for spawned sandbox processes.
//!
//! On Windows this is a job object owned by OpenTopia and wrapped around the process it
//! spawns, which for sandboxed execution is the `codex sandbox` helper.
//! The job is always created on Windows so closing OpenTopia also closes the command tree;
//! a memory quota is applied unless the caller supplies a different one.
//!
//! The helper creates a job object of its own, so this is deliberately an *outer* job. That
//! works because nested job limits intersect: a process bound by both jobs cannot exceed
//! either one. It is also deliberately not modelled on the helper's job. Verified against
//! `openai/codex` at `61a44880a`, the helper's job (`codex-rs/utils/pty/src/win/job.rs`)
//! carries no quota at all and sets `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, which lets a child leave
//! the job with `CREATE_BREAKAWAY_FROM_JOB`. That is correct for its purpose — reclaiming a
//! process tree — but it is the opposite of what a quota needs, so this job never sets
//! `BREAKAWAY_OK`.
//!
//! Two properties matter and are easy to get wrong:
//!
//! * The process is spawned suspended and only resumed after it has been assigned. Assigning
//!   a running process leaves a window in which it can spawn descendants outside the job, and
//!   those descendants would be unmetered. The helper assigns after spawn; this does not.
//! * A failed assignment is an error rather than a warning. Continuing would silently run the
//!   command with no quota, which is worse than refusing to run it.
//!
//! The job handle must outlive the process. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` terminates
//! every member when the last handle closes, so dropping [`ProcessQuota`] early kills the
//! command it was protecting. Callers hold it until the process exits.

use crate::execution::ResourceLimit;

#[cfg(windows)]
const FALLBACK_MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(windows)]
const MIN_DEFAULT_MEMORY_BYTES: u64 = 1 * 1024 * 1024 * 1024;
#[cfg(windows)]
const MAX_DEFAULT_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// Positive decimal bytes override the default; `0` keeps cleanup enabled but removes the
/// memory cap for troubleshooting or unusually large local builds.
#[cfg(windows)]
const MAX_MEMORY_ENV: &str = "OPENTOPIA_MAX_MEMORY_BYTES";

/// A resource quota bound to one spawned process tree.
///
/// Dropping this terminates the processes assigned to it.
#[derive(Debug)]
pub struct ProcessQuota {
    #[cfg(windows)]
    job: imp::OwnedJob,
}

impl ProcessQuota {
    /// Builds the Windows process-tree job for `limits`.
    ///
    /// On Windows this is always `Some`, even when no explicit CPU or memory limit is set,
    /// because the job also owns process-tree cleanup. An explicit memory limit wins over
    /// the dynamic default and the [`MAX_MEMORY_ENV`] override.
    ///
    /// On other platforms this remains `None`; `max_output_bytes` is enforced by the output
    /// reader everywhere.
    pub fn prepare(limits: &ResourceLimit) -> anyhow::Result<Option<Self>> {
        #[cfg(windows)]
        {
            let mut effective_limits = limits.clone();
            if effective_limits.max_memory_bytes.is_none() {
                effective_limits.max_memory_bytes = default_memory_limit_bytes();
            }
            Ok(Some(Self {
                job: imp::OwnedJob::create(&effective_limits)?,
            }))
        }
        #[cfg(not(windows))]
        {
            if limits.max_cpu_time.is_some() || limits.max_memory_bytes.is_some() {
                // Only Windows is in scope for the current delivery target. Refusing here would
                // break local development on other platforms, so the limit is reported as
                // unenforced instead of silently claimed.
                tracing::warn!(
                    "process CPU/memory quotas are only enforced on Windows; running unmetered"
                );
            }
            Ok(None)
        }
    }

    /// Assigns a process spawned with [`suspended_creation_flags`] and then resumes it.
    ///
    /// Fails closed: if the process cannot be assigned it is killed rather than allowed to
    /// run without the quota.
    #[cfg(windows)]
    pub fn bind_and_resume(&self, child: &tokio::process::Child) -> anyhow::Result<()> {
        let handle = child
            .raw_handle()
            .ok_or_else(|| anyhow::anyhow!("quota target has already exited"))?;
        let pid = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("quota target has already exited"))?;
        self.job.assign(handle)?;
        imp::resume_process(pid)
    }

    #[cfg(not(windows))]
    pub fn bind_and_resume(&self, _child: &tokio::process::Child) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
fn default_memory_limit_bytes() -> Option<u64> {
    let default_bytes = physical_memory_bytes()
        .map(default_memory_limit_for_total)
        .unwrap_or_else(|| {
            tracing::warn!(
                fallback_bytes = FALLBACK_MEMORY_BYTES,
                "could not determine physical memory; using the fallback process memory limit"
            );
            FALLBACK_MEMORY_BYTES
        });
    let configured = std::env::var(MAX_MEMORY_ENV).ok();
    resolve_memory_limit(None, configured.as_deref(), default_bytes)
}

#[cfg(windows)]
fn physical_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `status` is initialized with the required structure size and a valid pointer.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        None
    } else {
        Some(status.ullTotalPhys)
    }
}

#[cfg(windows)]
fn default_memory_limit_for_total(total_memory_bytes: u64) -> u64 {
    (total_memory_bytes / 2).clamp(MIN_DEFAULT_MEMORY_BYTES, MAX_DEFAULT_MEMORY_BYTES)
}

#[cfg(windows)]
fn resolve_memory_limit(
    explicit: Option<u64>,
    env_value: Option<&str>,
    default_bytes: u64,
) -> Option<u64> {
    if explicit.is_some() {
        return explicit;
    }

    match env_value {
        Some(value) if value.trim() == "0" => None,
        Some(value) => match parse_memory_limit(value) {
            Some(bytes) => Some(bytes),
            None => {
                tracing::warn!(
                    variable = MAX_MEMORY_ENV,
                    default_bytes,
                    "invalid memory limit; using the default"
                );
                Some(default_bytes)
            }
        },
        None => Some(default_bytes),
    }
}

#[cfg(windows)]
fn parse_memory_limit(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok().filter(|bytes| *bytes > 0)
}

/// Creation flags required so a process can be assigned before it runs any code.
///
/// Returns `0` on non-Windows or when no process-tree job is present, so the caller can pass
/// the result unconditionally.
#[cfg(windows)]
pub fn suspended_creation_flags(quota: Option<&ProcessQuota>) -> u32 {
    if quota.is_some() {
        windows_sys::Win32::System::Threading::CREATE_SUSPENDED
    } else {
        0
    }
}

#[cfg(windows)]
mod imp {
    use super::ResourceLimit;
    use anyhow::{anyhow, Context};
    use std::os::windows::io::RawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_JOB_MEMORY,
        JOB_OBJECT_LIMIT_JOB_TIME, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    /// Units of `PerJobUserTimeLimit`, which counts in 100-nanosecond intervals.
    const HUNDRED_NANOS_PER_SEC: u64 = 10_000_000;

    #[derive(Debug)]
    pub(super) struct OwnedJob(HANDLE);

    // The handle is owned exclusively and only used through the job APIs, which are
    // thread safe.
    unsafe impl Send for OwnedJob {}
    unsafe impl Sync for OwnedJob {}

    impl OwnedJob {
        pub(super) fn create(limits: &ResourceLimit) -> anyhow::Result<Self> {
            // SAFETY: null attributes and a null name request an unnamed job with default
            // security, and the returned handle is checked before use.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error()).context("CreateJobObjectW failed");
            }
            let job = Self(handle);

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION =
                unsafe { std::mem::zeroed::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() };
            // Reclaim the whole tree when the last handle closes. Without this a command
            // that outlives OpenTopia would keep running unsupervised.
            let mut flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            if let Some(memory) = limits.max_memory_bytes {
                if memory == 0 {
                    return Err(anyhow!("memory quota must be greater than zero"));
                }
                // Job-wide rather than per-process: the limit should cover the command and
                // everything it spawns, not reset for each descendant.
                flags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                info.JobMemoryLimit = usize::try_from(memory)
                    .map_err(|_| anyhow!("memory quota {memory} exceeds this platform's range"))?;
            }
            if let Some(cpu) = limits.max_cpu_time {
                flags |= JOB_OBJECT_LIMIT_JOB_TIME;
                let hundred_nanos = (cpu.as_secs_f64() * HUNDRED_NANOS_PER_SEC as f64).ceil();
                if !hundred_nanos.is_finite() || hundred_nanos <= 0.0 {
                    return Err(anyhow!("cpu quota {cpu:?} is not a usable duration"));
                }
                info.BasicLimitInformation.PerJobUserTimeLimit = hundred_nanos as i64;
            }
            // Never JOB_OBJECT_LIMIT_BREAKAWAY_OK: it would let a child leave the job with
            // CREATE_BREAKAWAY_FROM_JOB and escape the quota entirely.
            info.BasicLimitInformation.LimitFlags = flags;

            // SAFETY: `info` matches the class being set and its declared size.
            let ok = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("SetInformationJobObject failed");
            }
            Ok(job)
        }

        pub(super) fn assign(&self, process: RawHandle) -> anyhow::Result<()> {
            // SAFETY: `process` is a live process handle owned by the caller's `Child`.
            let ok = unsafe { AssignProcessToJobObject(self.0, process as HANDLE) };
            if ok == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("AssignProcessToJobObject failed");
            }
            Ok(())
        }
    }

    impl Drop for OwnedJob {
        fn drop(&mut self) {
            // SAFETY: the handle is owned and closed exactly once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct OwnedSnapshot(HANDLE);

    impl Drop for OwnedSnapshot {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Resumes every thread of `pid`.
    ///
    /// A process created with `CREATE_SUSPENDED` has exactly one thread, but neither `std`
    /// nor `tokio` exposes the primary thread handle from `PROCESS_INFORMATION`, so the
    /// threads are found through a toolhelp snapshot instead.
    pub(super) fn resume_process(pid: u32) -> anyhow::Result<()> {
        // SAFETY: requests a thread snapshot; the handle is validated before use.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error()).context("CreateToolhelp32Snapshot failed");
        }
        let snapshot = OwnedSnapshot(snapshot);

        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed::<THREADENTRY32>() };
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

        // SAFETY: `entry.dwSize` is set as the API requires.
        if unsafe { Thread32First(snapshot.0, &mut entry) } == 0 {
            return Err(std::io::Error::last_os_error()).context("Thread32First failed");
        }

        let mut resumed = 0usize;
        loop {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: opening a thread by id; the handle is checked and then closed.
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if !thread.is_null() {
                    // SAFETY: the handle was opened with THREAD_SUSPEND_RESUME.
                    let previous = unsafe { ResumeThread(thread) };
                    unsafe {
                        CloseHandle(thread);
                    }
                    if previous == u32::MAX {
                        return Err(std::io::Error::last_os_error()).context("ResumeThread failed");
                    }
                    resumed += 1;
                }
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            // SAFETY: same invariants as `Thread32First`.
            if unsafe { Thread32Next(snapshot.0, &mut entry) } == 0 {
                break;
            }
        }

        if resumed == 0 {
            // Leaving the process suspended would hang the caller on a read that never
            // completes, so this is an error rather than a warning.
            return Err(anyhow!("no suspended thread found for process {pid}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[cfg(not(windows))]
    #[test]
    fn no_limits_produce_no_quota_on_non_windows() {
        let quota = ProcessQuota::prepare(&ResourceLimit::default()).expect("prepare");
        assert!(quota.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn no_limits_still_create_a_cleanup_job() {
        let quota = ProcessQuota::prepare(&ResourceLimit::default()).expect("prepare");
        assert!(quota.is_some());
    }

    #[test]
    fn output_limit_alone_does_not_add_a_resource_limit() {
        // Output truncation is enforced while reading the pipes, not by the OS. Windows still
        // creates its cleanup job even though this limit does not add a resource quota.
        let limits = ResourceLimit {
            max_output_bytes: Some(1024),
            ..ResourceLimit::default()
        };
        #[cfg(windows)]
        assert!(ProcessQuota::prepare(&limits).expect("prepare").is_some());
        #[cfg(not(windows))]
        assert!(ProcessQuota::prepare(&limits).expect("prepare").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn memory_limit_configuration_is_positive_and_explicit_values_win() {
        assert_eq!(parse_memory_limit("134217728"), Some(128 * 1024 * 1024));
        assert_eq!(parse_memory_limit("0"), None);
        assert_eq!(parse_memory_limit("not-a-number"), None);
        assert_eq!(
            resolve_memory_limit(None, Some("134217728"), 4 * 1024 * 1024 * 1024),
            Some(128 * 1024 * 1024)
        );
        assert_eq!(
            resolve_memory_limit(
                Some(256 * 1024 * 1024),
                Some("134217728"),
                4 * 1024 * 1024 * 1024,
            ),
            Some(256 * 1024 * 1024)
        );
        assert_eq!(
            resolve_memory_limit(None, Some("0"), 4 * 1024 * 1024 * 1024),
            None
        );
        assert_eq!(
            resolve_memory_limit(None, Some("not-a-number"), 4 * 1024 * 1024 * 1024),
            Some(4 * 1024 * 1024 * 1024)
        );
        assert_eq!(
            default_memory_limit_for_total(4 * 1024 * 1024 * 1024),
            2 * 1024 * 1024 * 1024
        );
        assert_eq!(
            default_memory_limit_for_total(16 * 1024 * 1024 * 1024),
            MAX_DEFAULT_MEMORY_BYTES
        );
        assert_eq!(
            default_memory_limit_for_total(1 * 1024 * 1024 * 1024),
            MIN_DEFAULT_MEMORY_BYTES
        );
    }

    #[cfg(windows)]
    #[test]
    fn memory_limit_creates_a_job() {
        let limits = ResourceLimit {
            max_memory_bytes: Some(256 * 1024 * 1024),
            ..ResourceLimit::default()
        };
        assert!(ProcessQuota::prepare(&limits).expect("prepare").is_some());
    }

    #[cfg(windows)]
    #[test]
    fn cpu_limit_creates_a_job() {
        let limits = ResourceLimit {
            max_cpu_time: Some(Duration::from_secs(5)),
            ..ResourceLimit::default()
        };
        assert!(ProcessQuota::prepare(&limits).expect("prepare").is_some());
    }

    #[cfg(windows)]
    #[test]
    fn zero_cpu_limit_is_rejected_rather_than_silently_unbounded() {
        let limits = ResourceLimit {
            max_cpu_time: Some(Duration::ZERO),
            ..ResourceLimit::default()
        };
        assert!(ProcessQuota::prepare(&limits).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn zero_memory_limit_is_rejected_rather_than_silently_unbounded() {
        let limits = ResourceLimit {
            max_memory_bytes: Some(0),
            ..ResourceLimit::default()
        };
        assert!(ProcessQuota::prepare(&limits).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn suspended_flag_is_only_requested_when_a_quota_exists() {
        assert_eq!(suspended_creation_flags(None), 0);
        let limits = ResourceLimit {
            max_memory_bytes: Some(64 * 1024 * 1024),
            ..ResourceLimit::default()
        };
        let quota = ProcessQuota::prepare(&limits)
            .expect("prepare")
            .expect("quota");
        assert_ne!(suspended_creation_flags(Some(&quota)), 0);
    }
}
