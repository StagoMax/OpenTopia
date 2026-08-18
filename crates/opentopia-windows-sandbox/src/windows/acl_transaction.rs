use super::process_launch::{last_error, wide};
use super::security_token::SidBuffer;
use super::{normalized_capability_path, TRUSTEE_IS_UNKNOWN_VALUE, WRITE_RESTRICTION_PERMISSIONS};
use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, DENY_ACCESS, EXPLICIT_ACCESS_W,
    GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, PSID, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

pub(super) struct AclTransaction {
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
    pub(super) fn grant(
        &mut self,
        path: &Path,
        sid: PSID,
        inherit: bool,
        permissions: u32,
    ) -> Result<()> {
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid: SidBuffer::copy_from_sid(sid)?.0,
        });
        self.persist()?;
        apply_dacl_change(path, sid, GRANT_ACCESS, inherit, permissions)?;
        Ok(())
    }

    pub(super) fn deny_write(&mut self, path: &Path, sid: PSID, inherit: bool) -> Result<()> {
        self.deny(path, sid, inherit, WRITE_RESTRICTION_PERMISSIONS)
    }

    pub(super) fn deny(
        &mut self,
        path: &Path,
        sid: PSID,
        inherit: bool,
        permissions: u32,
    ) -> Result<()> {
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid: SidBuffer::copy_from_sid(sid)?.0,
        });
        self.persist()?;
        apply_dacl_change(path, sid, DENY_ACCESS, inherit, permissions)?;
        Ok(())
    }

    pub(super) fn commit(mut self) {
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

pub(super) fn recover_acl_transactions() -> Result<()> {
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

pub(super) fn update_dacl(
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

pub(super) struct NamedAclMutex(HANDLE);

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

    pub(super) fn acquire_paths<'a>(
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<Vec<Self>> {
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

    pub(super) fn acquire_metadata() -> Result<Self> {
        Self::acquire_named(
            "Local\\OpenTopiaSandboxAclLedger",
            5_000,
            "ACL ledger transaction",
        )
    }
}

pub(super) fn acl_authorization_domain(path: &Path) -> String {
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

#[cfg(test)]
mod tests {
    use super::acl_authorization_domain;
    use std::path::Path;

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
}
