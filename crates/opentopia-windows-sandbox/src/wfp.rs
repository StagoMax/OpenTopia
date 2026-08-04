use anyhow::{anyhow, Result};
use std::ffi::OsStr;
use std::mem::zeroed;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    LocalFree, FWP_E_ALREADY_EXISTS, FWP_E_FILTER_NOT_FOUND, FWP_E_NOT_FOUND, HANDLE, HLOCAL,
};
use windows_sys::Win32::NetworkManagement::WindowsFilteringPlatform::{
    FwpmEngineClose0, FwpmEngineOpen0, FwpmFilterAdd0, FwpmFilterDeleteByKey0, FwpmProviderAdd0,
    FwpmSubLayerAdd0, FwpmTransactionAbort0, FwpmTransactionBegin0, FwpmTransactionCommit0,
    FWPM_ACTION0, FWPM_ACTION0_0, FWPM_CONDITION_ALE_USER_ID, FWPM_DISPLAY_DATA0, FWPM_FILTER0,
    FWPM_FILTER0_0, FWPM_FILTER_CONDITION0, FWPM_FILTER_FLAG_PERSISTENT,
    FWPM_LAYER_ALE_AUTH_CONNECT_V4, FWPM_LAYER_ALE_AUTH_CONNECT_V6, FWPM_PROVIDER0,
    FWPM_PROVIDER_FLAG_PERSISTENT, FWPM_SESSION0, FWPM_SUBLAYER0, FWPM_SUBLAYER_FLAG_PERSISTENT,
    FWP_ACTION_BLOCK, FWP_ACTRL_MATCH_FILTER, FWP_BYTE_BLOB, FWP_CONDITION_VALUE0,
    FWP_CONDITION_VALUE0_0, FWP_EMPTY, FWP_MATCH_EQUAL, FWP_SECURITY_DESCRIPTOR_TYPE, FWP_VALUE0,
};
use windows_sys::Win32::Security::Authorization::{
    BuildExplicitAccessWithNameW, BuildSecurityDescriptorW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
};
use windows_sys::Win32::Security::PSECURITY_DESCRIPTOR;
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
    transaction.commit()
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
