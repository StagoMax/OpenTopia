use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::mem::zeroed;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, FWP_E_ALREADY_EXISTS, FWP_E_FILTER_NOT_FOUND, FWP_E_NOT_FOUND,
    FWP_E_PROVIDER_NOT_FOUND, FWP_E_SUBLAYER_NOT_FOUND, HANDLE, HLOCAL,
};
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
    FwpmEngineClose0, FwpmEngineOpen0, FwpmFilterAdd0, FwpmFilterDeleteByKey0, FwpmFilterGetByKey0,
    FwpmFilterGetSecurityInfoByKey0, FwpmFilterSetSecurityInfoByKey0, FwpmFreeMemory0,
    FwpmProviderAdd0, FwpmProviderDeleteByKey0, FwpmSubLayerAdd0, FwpmSubLayerDeleteByKey0,
    FwpmTransactionAbort0, FwpmTransactionBegin0, FwpmTransactionCommit0, FWPM_ACTION0,
    FWPM_ACTION0_0, FWPM_ACTRL_READ, FWPM_CONDITION_ALE_USER_ID, FWPM_DISPLAY_DATA0, FWPM_FILTER0,
    FWPM_FILTER0_0, FWPM_FILTER_CONDITION0, FWPM_FILTER_FLAG_PERSISTENT,
    FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6, FWPM_PROVIDER0,
    FWPM_PROVIDER_FLAG_PERSISTENT, FWPM_SESSION0, FWPM_SUBLAYER0, FWPM_SUBLAYER_FLAG_PERSISTENT,
    FWP_ACTION_BLOCK, FWP_ACTRL_MATCH_FILTER, FWP_BYTE_BLOB, FWP_CONDITION_VALUE0,
    FWP_CONDITION_VALUE0_0, FWP_EMPTY, FWP_MATCH_EQUAL, FWP_SECURITY_DESCRIPTOR_TYPE, FWP_VALUE0,
};
use windows_sys::Win32::Security::Authorization::{
    BuildExplicitAccessWithNameW, BuildSecurityDescriptorW, BuildTrusteeWithSidW, SetEntriesInAclW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS,
};
use windows_sys::Win32::Security::{
    CreateWellKnownSid, WinBuiltinUsersSid, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    SECURITY_MAX_SID_SIZE,
};
use windows_sys::Win32::System::Rpc::RPC_C_AUTHN_DEFAULT;
use windows_sys::Win32::System::Threading::INFINITE;

const PROVIDER_KEY: GUID = GUID::from_u128(0x65200f94_e04b_48cc_aefa_836cd1ea7201);
const SUBLAYER_KEY: GUID = GUID::from_u128(0xa1b81774_6004_475b_8654_50ce711183af);
const FILTER_V4_KEY: GUID = GUID::from_u128(0x8ab672a6_82d2_4b52_b899_4cc68428b52e);
const FILTER_V6_KEY: GUID = GUID::from_u128(0x4c985d0c_865c_4877_aac1_76f14a3ae65e);

pub(crate) fn install_offline_filters(account: &str) -> Result<()> {
    let engine = Engine::open()?;
    let transaction = Transaction::begin(&engine)?;
    ensure_provider(engine.handle)?;
    ensure_sublayer(engine.handle)?;
    let user = UserCondition::new(account)?;
    install_filter(
        engine.handle,
        &FILTER_V4_KEY,
        "OpenTopia sandbox offline IPv4",
        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
        &user,
    )?;
    install_filter(
        engine.handle,
        &FILTER_V6_KEY,
        "OpenTopia sandbox offline IPv6",
        FWPM_LAYER_ALE_AUTH_CONNECT_V6,
        &user,
    )?;
    transaction.commit()?;
    // WFP objects inherit an administrator-only read ACL by default. Runtime
    // health checks run without elevation, so grant the local Users group only
    // the object-level read right after the transaction commits. WFP security
    // descriptors cannot be changed from inside an explicit transaction.
    grant_filter_read_access(engine.handle, &FILTER_V4_KEY)?;
    grant_filter_read_access(engine.handle, &FILTER_V6_KEY)
}

pub(crate) fn offline_filters_installed() -> Result<bool> {
    let engine = Engine::open()?;
    Ok(filter_exists(engine.handle, &FILTER_V4_KEY)?
        && filter_exists(engine.handle, &FILTER_V6_KEY)?)
}

pub(crate) fn remove_offline_filters() -> Result<()> {
    let engine = Engine::open()?;
    let transaction = Transaction::begin(&engine)?;
    delete_filter_if_present(engine.handle, &FILTER_V4_KEY)?;
    delete_filter_if_present(engine.handle, &FILTER_V6_KEY)?;
    check_allowed(
        unsafe { FwpmSubLayerDeleteByKey0(engine.handle, &SUBLAYER_KEY) },
        "FwpmSubLayerDeleteByKey0",
        &[FWP_E_NOT_FOUND as u32, FWP_E_SUBLAYER_NOT_FOUND as u32],
    )?;
    check_allowed(
        unsafe { FwpmProviderDeleteByKey0(engine.handle, &PROVIDER_KEY) },
        "FwpmProviderDeleteByKey0",
        &[FWP_E_NOT_FOUND as u32, FWP_E_PROVIDER_NOT_FOUND as u32],
    )?;
    transaction.commit()
}

fn filter_exists(engine: HANDLE, key: &GUID) -> Result<bool> {
    let mut filter: *mut FWPM_FILTER0 = null_mut();
    let result = unsafe { FwpmFilterGetByKey0(engine, key, &mut filter) };
    if result == FWP_E_FILTER_NOT_FOUND as u32 || result == FWP_E_NOT_FOUND as u32 {
        return Ok(false);
    }
    check(result, "FwpmFilterGetByKey0")?;
    if !filter.is_null() {
        unsafe { FwpmFreeMemory0((&mut filter as *mut *mut FWPM_FILTER0).cast()) };
    }
    Ok(true)
}

fn grant_filter_read_access(engine: HANDLE, key: &GUID) -> Result<()> {
    let mut existing_dacl: *mut ACL = null_mut();
    let mut security_descriptor: PSECURITY_DESCRIPTOR = null_mut();
    check(
        unsafe {
            FwpmFilterGetSecurityInfoByKey0(
                engine,
                key,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut existing_dacl,
                null_mut(),
                &mut security_descriptor,
            )
        },
        "FwpmFilterGetSecurityInfoByKey0",
    )?;

    let mut updated_dacl: *mut ACL = null_mut();
    let result = (|| {
        // A null DACL already allows all access and therefore needs no new ACE.
        if existing_dacl.is_null() {
            return Ok(());
        }
        let mut users_sid = [0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut users_sid_size = users_sid.len() as u32;
        let created = unsafe {
            CreateWellKnownSid(
                WinBuiltinUsersSid,
                null_mut(),
                users_sid.as_mut_ptr().cast(),
                &mut users_sid_size,
            )
        };
        if created == 0 {
            return Err(anyhow!(
                "CreateWellKnownSid(WinBuiltinUsersSid) failed with Windows error {}",
                unsafe { GetLastError() }
            ));
        }

        let mut access: EXPLICIT_ACCESS_W = unsafe { zeroed() };
        access.grfAccessPermissions = FWPM_ACTRL_READ;
        access.grfAccessMode = GRANT_ACCESS;
        unsafe {
            BuildTrusteeWithSidW(&mut access.Trustee, users_sid.as_mut_ptr().cast());
        }
        check(
            unsafe { SetEntriesInAclW(1, &access, existing_dacl, &mut updated_dacl) },
            "SetEntriesInAclW(WFP read access)",
        )?;
        check(
            unsafe {
                FwpmFilterSetSecurityInfoByKey0(
                    engine,
                    key,
                    DACL_SECURITY_INFORMATION,
                    null(),
                    null(),
                    updated_dacl,
                    null(),
                )
            },
            "FwpmFilterSetSecurityInfoByKey0",
        )
    })();

    if !updated_dacl.is_null() {
        unsafe { LocalFree(updated_dacl as HLOCAL) };
    }
    if !security_descriptor.is_null() {
        unsafe { FwpmFreeMemory0(&mut security_descriptor) };
    }
    result
}

fn delete_filter_if_present(engine: HANDLE, key: &GUID) -> Result<()> {
    check_allowed(
        unsafe { FwpmFilterDeleteByKey0(engine, key) },
        "FwpmFilterDeleteByKey0",
        &[FWP_E_FILTER_NOT_FOUND as u32, FWP_E_NOT_FOUND as u32],
    )
}

struct Engine {
    handle: HANDLE,
}

impl Engine {
    fn open() -> Result<Self> {
        let name = wide("OpenTopia Windows sandbox setup");
        let mut session: FWPM_SESSION0 = unsafe { zeroed() };
        session.displayData = FWPM_DISPLAY_DATA0 {
            name: name.as_ptr() as *mut u16,
            description: null_mut(),
        };
        session.txnWaitTimeoutInMSec = INFINITE;
        let mut handle = null_mut();
        let result = unsafe {
            FwpmEngineOpen0(
                null(),
                RPC_C_AUTHN_DEFAULT as u32,
                null(),
                &session,
                &mut handle,
            )
        };
        check(result, "FwpmEngineOpen0")?;
        Ok(Self { handle })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { FwpmEngineClose0(self.handle) };
    }
}

struct Transaction<'a> {
    engine: &'a Engine,
    committed: bool,
}

impl<'a> Transaction<'a> {
    fn begin(engine: &'a Engine) -> Result<Self> {
        check(
            unsafe { FwpmTransactionBegin0(engine.handle, 0) },
            "FwpmTransactionBegin0",
        )?;
        Ok(Self {
            engine,
            committed: false,
        })
    }

    fn commit(mut self) -> Result<()> {
        check(
            unsafe { FwpmTransactionCommit0(self.engine.handle) },
            "FwpmTransactionCommit0",
        )?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.committed {
            unsafe { FwpmTransactionAbort0(self.engine.handle) };
        }
    }
}

struct UserCondition {
    descriptor: PSECURITY_DESCRIPTOR,
    blob: FWP_BYTE_BLOB,
}

impl UserCondition {
    fn new(account: &str) -> Result<Self> {
        let mut account_w = wide(account);
        let mut access: EXPLICIT_ACCESS_W = unsafe { zeroed() };
        unsafe {
            BuildExplicitAccessWithNameW(
                &mut access,
                account_w.as_mut_ptr(),
                FWP_ACTRL_MATCH_FILTER,
                GRANT_ACCESS,
                0,
            );
        }
        let mut descriptor = null_mut();
        let mut descriptor_len = 0;
        check(
            unsafe {
                BuildSecurityDescriptorW(
                    null(),
                    null(),
                    1,
                    &access,
                    0,
                    null(),
                    null_mut(),
                    &mut descriptor_len,
                    &mut descriptor,
                )
            },
            "BuildSecurityDescriptorW",
        )?;
        Ok(Self {
            descriptor,
            blob: FWP_BYTE_BLOB {
                size: descriptor_len,
                data: descriptor as *mut u8,
            },
        })
    }
}

impl Drop for UserCondition {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe { LocalFree(self.descriptor as HLOCAL) };
        }
    }
}

fn ensure_provider(engine: HANDLE) -> Result<()> {
    let name = wide("OpenTopia Windows sandbox");
    let provider = FWPM_PROVIDER0 {
        providerKey: PROVIDER_KEY,
        displayData: FWPM_DISPLAY_DATA0 {
            name: name.as_ptr() as *mut u16,
            description: null_mut(),
        },
        flags: FWPM_PROVIDER_FLAG_PERSISTENT,
        providerData: empty_blob(),
        serviceName: null_mut(),
    };
    check_allowed(
        unsafe { FwpmProviderAdd0(engine, &provider, null_mut()) },
        "FwpmProviderAdd0",
        &[FWP_E_ALREADY_EXISTS as u32],
    )
}

fn ensure_sublayer(engine: HANDLE) -> Result<()> {
    let name = wide("OpenTopia Windows sandbox");
    let provider_key = PROVIDER_KEY;
    let sublayer = FWPM_SUBLAYER0 {
        subLayerKey: SUBLAYER_KEY,
        displayData: FWPM_DISPLAY_DATA0 {
            name: name.as_ptr() as *mut u16,
            description: null_mut(),
        },
        flags: FWPM_SUBLAYER_FLAG_PERSISTENT,
        providerKey: &provider_key as *const GUID as *mut GUID,
        providerData: empty_blob(),
        weight: 0x8000,
    };
    check_allowed(
        unsafe { FwpmSubLayerAdd0(engine, &sublayer, null_mut()) },
        "FwpmSubLayerAdd0",
        &[FWP_E_ALREADY_EXISTS as u32],
    )
}

fn install_filter(
    engine: HANDLE,
    key: &GUID,
    display_name: &str,
    layer: GUID,
    user: &UserCondition,
) -> Result<()> {
    check_allowed(
        unsafe { FwpmFilterDeleteByKey0(engine, key) },
        "FwpmFilterDeleteByKey0",
        &[FWP_E_FILTER_NOT_FOUND as u32, FWP_E_NOT_FOUND as u32],
    )?;
    let name = wide(display_name);
    let provider_key = PROVIDER_KEY;
    let mut condition = FWPM_FILTER_CONDITION0 {
        fieldKey: FWPM_CONDITION_ALE_USER_ID,
        matchType: FWP_MATCH_EQUAL,
        conditionValue: FWP_CONDITION_VALUE0 {
            r#type: FWP_SECURITY_DESCRIPTOR_TYPE,
            Anonymous: FWP_CONDITION_VALUE0_0 {
                sd: &user.blob as *const FWP_BYTE_BLOB as *mut FWP_BYTE_BLOB,
            },
        },
    };
    let filter = FWPM_FILTER0 {
        filterKey: *key,
        displayData: FWPM_DISPLAY_DATA0 {
            name: name.as_ptr() as *mut u16,
            description: null_mut(),
        },
        flags: FWPM_FILTER_FLAG_PERSISTENT,
        providerKey: &provider_key as *const GUID as *mut GUID,
        providerData: empty_blob(),
        layerKey: layer,
        subLayerKey: SUBLAYER_KEY,
        weight: empty_value(),
        numFilterConditions: 1,
        filterCondition: &mut condition,
        action: FWPM_ACTION0 {
            r#type: FWP_ACTION_BLOCK,
            Anonymous: FWPM_ACTION0_0 {
                filterType: GUID::from_u128(0),
            },
        },
        Anonymous: FWPM_FILTER0_0 { rawContext: 0 },
        reserved: null_mut(),
        filterId: 0,
        effectiveWeight: empty_value(),
    };
    let mut filter_id = 0;
    check(
        unsafe { FwpmFilterAdd0(engine, &filter, null_mut(), &mut filter_id) },
        "FwpmFilterAdd0",
    )
}

fn empty_blob() -> FWP_BYTE_BLOB {
    FWP_BYTE_BLOB {
        size: 0,
        data: null_mut(),
    }
}

fn empty_value() -> FWP_VALUE0 {
    FWP_VALUE0 {
        r#type: FWP_EMPTY,
        Anonymous: unsafe { zeroed() },
    }
}

fn check(result: u32, operation: &str) -> Result<()> {
    check_allowed(result, operation, &[])
}

fn check_allowed(result: u32, operation: &str, allowed: &[u32]) -> Result<()> {
    if result == 0 || allowed.contains(&result) {
        Ok(())
    } else {
        Err(anyhow!("{operation} failed with WFP status 0x{result:08X}"))
    }
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
