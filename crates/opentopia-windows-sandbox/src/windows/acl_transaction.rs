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
    GetExplicitEntriesFromAclW, GetNamedSecurityInfoW, ProgressInvokeNever, SetEntriesInAclW,
    SetNamedSecurityInfoW, TreeResetNamedSecurityInfoW, DENY_ACCESS, EXPLICIT_ACCESS_W,
    GRANT_ACCESS, REVOKE_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    EqualSid, InitializeSecurityDescriptor, SetFileSecurityW, SetSecurityDescriptorDacl,
    DACL_SECURITY_INFORMATION, PSID, SECURITY_DESCRIPTOR, SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
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

    pub(super) fn grant_without_child_propagation(
        &mut self,
        path: &Path,
        sid: PSID,
        permissions: u32,
    ) -> Result<()> {
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid: SidBuffer::copy_from_sid(sid)?.0,
        });
        self.persist()?;
        update_dacl_without_child_propagation(path, sid, GRANT_ACCESS, permissions)
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
        let temporary = self
            .journal_path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec_pretty(&self.changes)?)
            .with_context(|| format!("write ACL recovery journal {}", temporary.display()))?;
        let temporary_w = wide(temporary.as_os_str());
        let journal_w = wide(self.journal_path.as_os_str());
        let published = unsafe {
            MoveFileExW(
                temporary_w.as_ptr(),
                journal_w.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if published == 0 {
            let _ = std::fs::remove_file(&temporary);
            return Err(last_error("publish ACL recovery journal with MoveFileExW"));
        }
        Ok(())
    }
}

impl Drop for AclTransaction {
    fn drop(&mut self) {
        let mut recovered = true;
        for change in self.changes.iter().rev() {
            let mut sid = SidBuffer(change.sid.clone());
            recovered &=
                update_dacl_without_child_propagation(&change.path, sid.as_ptr(), REVOKE_ACCESS, 0)
                    .is_ok();
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
        let Some(changes) = read_acl_journal(&path)? else {
            continue;
        };
        let _guards =
            NamedAclMutex::acquire_paths(changes.iter().map(|change| change.path.as_path()))?;
        // A live transaction holds the same path locks. It can commit and
        // remove its journal while recovery waits, so re-read only after the
        // locks are owned. Missing then means another process completed the
        // transaction or its recovery; both are successful outcomes.
        let Some(changes) = read_acl_journal(&path)? else {
            continue;
        };
        for change in changes.iter().rev() {
            let mut sid = SidBuffer(change.sid.clone());
            // Remove the explicit scope-root ACE without recursively walking a
            // potentially huge workspace during command startup. Partial
            // inherited ACEs carry the same scope-specific SID and are inert
            // outside that policy; the next provision reconciles the root and
            // its object generation authoritatively.
            update_dacl_without_child_propagation(&change.path, sid.as_ptr(), REVOKE_ACCESS, 0)
                .with_context(|| {
                    format!("recover interrupted ACL transaction {}", path.display())
                })?;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("remove recovered ACL journal {}", path.display()))
            }
        }
    }
    Ok(())
}

fn read_acl_journal(path: &Path) -> Result<Option<Vec<AclChange>>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read ACL recovery journal {}", path.display()))
        }
    };
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse ACL recovery journal {}", path.display()))
        .map(Some)
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

pub(super) fn dacl_has_explicit_access(
    path: &Path,
    sid: PSID,
    access_mode: i32,
    permissions: u32,
) -> Result<bool> {
    let name = wide(path.as_os_str());
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let get_result = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if get_result != 0 {
        anyhow::bail!(
            "GetNamedSecurityInfoW failed while inspecting {}: {get_result}",
            path.display()
        )
    }
    let mut count = 0;
    let mut entries = ptr::null_mut();
    let entries_result = if dacl.is_null() {
        0
    } else {
        unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) }
    };
    let found = if entries_result == 0 && !entries.is_null() {
        unsafe { std::slice::from_raw_parts(entries, count as usize) }
            .iter()
            .any(|entry| {
                entry.grfAccessMode == access_mode
                    && entry.grfAccessPermissions & permissions == permissions
                    && entry.Trustee.TrusteeForm == TRUSTEE_IS_SID
                    && !entry.Trustee.ptstrName.is_null()
                    && unsafe { EqualSid(entry.Trustee.ptstrName.cast(), sid) } != 0
            })
    } else {
        false
    };
    if !entries.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(entries.cast()) };
    }
    if !descriptor.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(descriptor.cast()) };
    }
    if entries_result != 0 {
        anyhow::bail!(
            "GetExplicitEntriesFromAclW failed while inspecting {}: {entries_result}",
            path.display()
        )
    }
    Ok(found)
}

/// Update only the named directory. `SetNamedSecurityInfoW` automatically
/// propagates inheritable ACEs from a container's complete DACL, which can
/// turn a one-directory traversal grant into a scan of an entire user profile.
/// `SetFileSecurityW` applies this absolute descriptor without walking child
/// objects; the new ACE itself is deliberately non-inheriting.
fn update_dacl_without_child_propagation(
    path: &Path,
    sid: PSID,
    access_mode: i32,
    permissions: u32,
) -> Result<()> {
    let name = wide(path.as_os_str());
    let mut old_dacl = ptr::null_mut();
    let mut source_descriptor = ptr::null_mut();
    let get_result = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut old_dacl,
            ptr::null_mut(),
            &mut source_descriptor,
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
        grfInheritance: 0,
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
    if !source_descriptor.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(source_descriptor.cast()) };
    }
    if entries_result != 0 {
        anyhow::bail!(
            "SetEntriesInAclW failed for {}: {entries_result}",
            path.display()
        )
    }

    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor_ptr: *mut std::ffi::c_void =
        (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    let applied = (|| -> Result<()> {
        if unsafe { InitializeSecurityDescriptor(descriptor_ptr, 1) } == 0 {
            return Err(last_error("InitializeSecurityDescriptor"));
        }
        if unsafe { SetSecurityDescriptorDacl(descriptor_ptr, 1, new_dacl, 0) } == 0 {
            return Err(last_error("SetSecurityDescriptorDacl"));
        }
        if unsafe { SetFileSecurityW(name.as_ptr(), DACL_SECURITY_INFORMATION, descriptor_ptr) }
            == 0
        {
            return Err(last_error("SetFileSecurityW without child propagation"));
        }
        Ok(())
    })();
    if !new_dacl.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(new_dacl.cast()) };
    }
    applied
}

/// Rebuild descendant inherited ACEs from the directory's current DACL while
/// preserving explicit child ACLs. This is reserved for one-time identity
/// migrations; steady-state provisioning only updates capability roots.
pub(super) fn propagate_inherited_dacl(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Ok(());
    }
    let name = wide(path.as_os_str());
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let get_result = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if get_result != 0 {
        anyhow::bail!(
            "GetNamedSecurityInfoW failed before ACL propagation for {}: {get_result}",
            path.display()
        )
    }
    let reset_result = unsafe {
        TreeResetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
            1,
            None,
            ProgressInvokeNever,
            ptr::null(),
        )
    };
    if !descriptor.is_null() {
        unsafe { windows_sys::Win32::Foundation::LocalFree(descriptor.cast()) };
    }
    if reset_result != 0 {
        anyhow::bail!(
            "TreeResetNamedSecurityInfoW failed for {}: {reset_result}",
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
