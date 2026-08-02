use super::NetworkMode;
use super::SandboxRequest;
use anyhow::Result;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
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
use windows_sys::Win32::Security::FreeSid;
use windows_sys::Win32::Security::GetLengthSid;
use windows_sys::Win32::Security::GetTokenInformation;
use windows_sys::Win32::Security::Isolation::CreateAppContainerProfile;
use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;
use windows_sys::Win32::Security::Isolation::DeriveAppContainerSidFromAppContainerName;
use windows_sys::Win32::Security::SetTokenInformation;
use windows_sys::Win32::Security::TokenDefaultDacl;
use windows_sys::Win32::Security::TokenGroups;
use windows_sys::Win32::Security::WinCapabilityInternetClientSid;
use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
use windows_sys::Win32::Security::DISABLE_MAX_PRIVILEGE;
use windows_sys::Win32::Security::LUA_TOKEN;
use windows_sys::Win32::Security::PSID;
use windows_sys::Win32::Security::SECURITY_CAPABILITIES;
use windows_sys::Win32::Security::SID_AND_ATTRIBUTES;
use windows_sys::Win32::Security::SUB_CONTAINERS_AND_OBJECTS_INHERIT;
use windows_sys::Win32::Security::TOKEN_ADJUST_DEFAULT;
use windows_sys::Win32::Security::TOKEN_ADJUST_PRIVILEGES;
use windows_sys::Win32::Security::TOKEN_ADJUST_SESSIONID;
use windows_sys::Win32::Security::TOKEN_ASSIGN_PRIMARY;
use windows_sys::Win32::Security::TOKEN_DUPLICATE;
use windows_sys::Win32::Security::TOKEN_QUERY;
use windows_sys::Win32::Security::WRITE_RESTRICTED;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_EXECUTE;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_WRITE;
use windows_sys::Win32::System::Console::GetStdHandle;
use windows_sys::Win32::System::Console::STD_ERROR_HANDLE;
use windows_sys::Win32::System::Console::STD_INPUT_HANDLE;
use windows_sys::Win32::System::Console::STD_OUTPUT_HANDLE;
use windows_sys::Win32::System::JobObjects::CreateJobObjectW;
use windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation;
use windows_sys::Win32::System::JobObjects::SetInformationJobObject;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
use windows_sys::Win32::System::Threading::CreateProcessAsUserW;
use windows_sys::Win32::System::Threading::DeleteProcThreadAttributeList;
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::Threading::GetExitCodeProcess;
use windows_sys::Win32::System::Threading::InitializeProcThreadAttributeList;
use windows_sys::Win32::System::Threading::OpenProcessToken;
use windows_sys::Win32::System::Threading::UpdateProcThreadAttribute;
use windows_sys::Win32::System::Threading::WaitForSingleObject;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::System::Threading::CREATE_UNICODE_ENVIRONMENT;
use windows_sys::Win32::System::Threading::EXTENDED_STARTUPINFO_PRESENT;
use windows_sys::Win32::System::Threading::INFINITE;
use windows_sys::Win32::System::Threading::LPPROC_THREAD_ATTRIBUTE_LIST;
use windows_sys::Win32::System::Threading::PROCESS_INFORMATION;
use windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_HANDLE_LIST;
use windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_JOB_LIST;
use windows_sys::Win32::System::Threading::PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES;
use windows_sys::Win32::System::Threading::STARTF_USESTDHANDLES;
use windows_sys::Win32::System::Threading::STARTUPINFOEXW;
const TRUSTEE_IS_UNKNOWN_VALUE: i32 = TRUSTEE_IS_UNKNOWN;
const SE_GROUP_LOGON_ID: u32 = 0xC000_0000;
const WIN_WORLD_SID: i32 = 1;
const GENERIC_ALL: u32 = 0x1000_0000;

#[repr(C)]
struct TokenDefaultDaclInfo {
    default_dacl: *mut windows_sys::Win32::Security::ACL,
}

pub(super) fn run(request: SandboxRequest) -> Result<i32> {
    let mut app_container = AppContainer::create(request.network)?;
    let app_container_sid = app_container.sid;
    let capability_sid = app_container.sandbox_capability();
    let mut acl = AclTransaction::default();
    for root in &request.read_roots {
        acl.grant(
            root,
            app_container_sid,
            false,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )?;
        acl.grant(
            root,
            capability_sid,
            false,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )?;
    }
    for root in &request.write_roots {
        acl.grant(
            root,
            app_container_sid,
            true,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | FILE_GENERIC_WRITE,
        )?;
        acl.grant(
            root,
            capability_sid,
            true,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | FILE_GENERIC_WRITE,
        )?;
    }
    for path in &request.protected_paths {
        if path.exists() {
            acl.deny_write(path, capability_sid)?;
        }
    }

    let restricted_token = RestrictedToken::for_capability(capability_sid)?;
    let exit_code = launch(&request, &mut app_container, restricted_token.handle)?;
    drop(acl);
    Ok(exit_code as i32)
}

struct AppContainer {
    name: Vec<u16>,
    sid: PSID,
    capability: SidBuffer,
    capabilities: Vec<SidBuffer>,
    created: bool,
}

impl AppContainer {
    fn create(network: NetworkMode) -> Result<Self> {
        let name = format!("OpenTopia.Sandbox.{}", Uuid::new_v4().simple());
        let name_wide = wide(&name);
        let mut sid = ptr::null_mut();
        let result = unsafe {
            CreateAppContainerProfile(
                name_wide.as_ptr(),
                name_wide.as_ptr(),
                name_wide.as_ptr(),
                ptr::null(),
                0,
                &mut sid,
            )
        };
        let created = result >= 0;
        if !created {
            let derive =
                unsafe { DeriveAppContainerSidFromAppContainerName(name_wide.as_ptr(), &mut sid) };
            if derive < 0 || sid.is_null() {
                anyhow::bail!("CreateAppContainerProfile failed: 0x{result:08X}")
            }
        }

        let capability = SidBuffer::random_capability();
        let capabilities = match network {
            NetworkMode::Deny => Vec::new(),
            NetworkMode::Internet => vec![SidBuffer::well_known(WinCapabilityInternetClientSid)?],
        };
        Ok(Self {
            name: name_wide,
            sid,
            capability,
            capabilities,
            created,
        })
    }

    fn launch_capabilities(&mut self) -> LaunchCapabilities {
        let mut entries = self
            .capabilities
            .iter_mut()
            .map(SidBuffer::as_sid_and_attributes)
            .collect::<Vec<_>>();
        let value = SECURITY_CAPABILITIES {
            AppContainerSid: self.sid,
            Capabilities: (!entries.is_empty())
                .then_some(entries.as_mut_ptr())
                .unwrap_or(ptr::null_mut()),
            CapabilityCount: entries.len() as u32,
            Reserved: 0,
        };
        LaunchCapabilities {
            _entries: entries,
            value,
        }
    }

    fn sandbox_capability(&mut self) -> PSID {
        self.capability.as_ptr()
    }
}

struct LaunchCapabilities {
    _entries: Vec<SID_AND_ATTRIBUTES>,
    value: SECURITY_CAPABILITIES,
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
        let mut restricting_sids = vec![unsafe { std::mem::zeroed::<SID_AND_ATTRIBUTES>() }; 3];
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

impl Drop for AppContainer {
    fn drop(&mut self) {
        unsafe {
            if self.created {
                let _ = DeleteAppContainerProfile(self.name.as_ptr());
            }
            if !self.sid.is_null() {
                FreeSid(self.sid);
            }
        }
    }
}

struct SidBuffer(Vec<u8>);

impl SidBuffer {
    fn random_capability() -> Self {
        let random = *Uuid::new_v4().as_bytes();
        let values = [
            21_u32,
            u32::from_le_bytes(random[0..4].try_into().expect("uuid segment")),
            u32::from_le_bytes(random[4..8].try_into().expect("uuid segment")),
            u32::from_le_bytes(random[8..12].try_into().expect("uuid segment")),
            u32::from_le_bytes(random[12..16].try_into().expect("uuid segment")),
        ];
        let mut bytes = Vec::with_capacity(8 + values.len() * 4);
        bytes.extend([1, values.len() as u8, 0, 0, 0, 0, 0, 5]);
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
        Self(bytes)
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

    fn deny_write(&mut self, path: &Path, sid: PSID) -> Result<()> {
        update_dacl(path, sid, DENY_ACCESS, false, FILE_GENERIC_WRITE)?;
        self.changes.push(AclChange {
            path: path.to_path_buf(),
            sid,
        });
        Ok(())
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

fn launch(
    request: &SandboxRequest,
    app_container: &mut AppContainer,
    restricted_token: HANDLE,
) -> Result<u32> {
    let job = create_job()?;
    let mut launch_capabilities = app_container.launch_capabilities();
    let mut attributes = AttributeList::new(3)?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
        (&mut launch_capabilities.value as *mut SECURITY_CAPABILITIES).cast(),
        std::mem::size_of::<SECURITY_CAPABILITIES>(),
    )?;
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
    let created = unsafe {
        CreateProcessAsUserW(
            restricted_token,
            ptr::null(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
            ptr::null(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessW"));
    }
    unsafe { CloseHandle(process_info.hThread) };
    let waited = unsafe { WaitForSingleObject(process_info.hProcess, INFINITE) };
    if waited == u32::MAX {
        unsafe { CloseHandle(process_info.hProcess) };
        return Err(last_error("WaitForSingleObject"));
    }
    let mut exit_code = 1;
    let code = unsafe { GetExitCodeProcess(process_info.hProcess, &mut exit_code) };
    unsafe { CloseHandle(process_info.hProcess) };
    if code == 0 {
        return Err(last_error("GetExitCodeProcess"));
    }
    unsafe { CloseHandle(job) };
    Ok(exit_code)
}

fn create_job() -> Result<HANDLE> {
    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() || job == INVALID_HANDLE_VALUE {
        return Err(last_error("CreateJobObjectW"));
    }
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
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
