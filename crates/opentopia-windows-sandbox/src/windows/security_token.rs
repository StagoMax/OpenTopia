use super::process_launch::{last_error, wide};
use super::{
    TokenDefaultDaclInfo, GENERIC_ALL, SE_GROUP_LOGON_ID, TRUSTEE_IS_UNKNOWN_VALUE, WIN_WORLD_SID,
};
use anyhow::Result;
use std::ffi::OsStr;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT,
    TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AccessCheck, CopySid, CreateRestrictedToken, CreateWellKnownSid, DuplicateToken, GetLengthSid,
    GetTokenInformation, LogonUserW, MapGenericMask, SecurityImpersonation, SetTokenInformation,
    TokenDefaultDacl, TokenGroups, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE,
    GENERIC_MAPPING, GROUP_SECURITY_INFORMATION, LOGON32_LOGON_INTERACTIVE,
    LOGON32_PROVIDER_DEFAULT, LUA_TOKEN, OWNER_SECURITY_INFORMATION, PRIVILEGE_SET, PSID,
    SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES, TOKEN_ADJUST_SESSIONID,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY, WRITE_RESTRICTED,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub(super) struct LoggedOnToken {
    pub(super) handle: HANDLE,
}

impl LoggedOnToken {
    pub(super) fn new(username: &str, password: &str) -> Result<Self> {
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
pub(super) fn effective_file_access(
    token: HANDLE,
    path: &Path,
    desired_access: u32,
) -> Result<bool> {
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

pub(super) struct RestrictedToken {
    pub(super) handle: HANDLE,
}

impl RestrictedToken {
    pub(super) fn for_capability(capability_sid: PSID) -> Result<Self> {
        Self::create(capability_sid, None)
    }

    pub(super) fn for_dedicated_capability(capability_sid: PSID) -> Result<Self> {
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

pub(super) struct SidBuffer(pub(super) Vec<u8>);

impl SidBuffer {
    pub(super) fn opentopia_capability(id: Uuid) -> Self {
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

    pub(super) fn legacy_opentopia_capability() -> Self {
        Self::opentopia_capability(Uuid::from_bytes(*b"OpenTopiaSandbox"))
    }

    pub(super) fn opentopia_runtime() -> Self {
        Self::opentopia_capability(Uuid::from_bytes(*b"OpenTopiaRuntime"))
    }

    pub(super) fn well_known(kind: i32) -> Result<Self> {
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

    pub(super) fn copy_from_sid(sid: PSID) -> Result<Self> {
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

    pub(super) fn as_ptr(&mut self) -> PSID {
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

#[cfg(test)]
mod tests {
    #[test]
    fn dedicated_restricted_token_keeps_runtime_compatibility_sids_explicit() {
        let source = include_str!("security_token.rs");
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
}
