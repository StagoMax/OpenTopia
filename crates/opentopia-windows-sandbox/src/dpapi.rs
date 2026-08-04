use anyhow::{anyhow, Result};
use windows_sys::Win32::Foundation::{GetLastError, LocalFree, HLOCAL};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
    CRYPT_INTEGER_BLOB,
};

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    }
}

pub(crate) fn protect(data: &[u8]) -> Result<Vec<u8>> {
    let input = blob(data);
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN | CRYPTPROTECT_LOCAL_MACHINE,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(anyhow!("CryptProtectData failed: {}", unsafe {
            GetLastError()
        }));
    }
    let protected =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        if !output.pbData.is_null() {
            LocalFree(output.pbData as HLOCAL);
        }
    }
    Ok(protected)
}

pub(crate) fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    let input = blob(data);
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(anyhow!("CryptUnprotectData failed: {}", unsafe {
            GetLastError()
        }));
    }
    let unprotected =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        if !output.pbData.is_null() {
            LocalFree(output.pbData as HLOCAL);
        }
    }
    Ok(unprotected)
}
