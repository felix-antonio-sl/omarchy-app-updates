pub(crate) mod bindings;
mod callbacks;
mod config;
mod read;
pub mod transaction;
use crate::domain::*;
pub use config::*;
pub use read::*;
use std::{
    cmp::Ordering,
    ffi::{CStr, CString},
};
pub fn validate_abi(compiled: &str, runtime: &str) -> AppResult<()> {
    let major = |s: &str| s.split('.').next().and_then(|n| n.parse::<u32>().ok());
    if major(compiled).is_none() || major(compiled) != major(runtime) {
        return Err(AppError::new(
            ErrorCode::AbiIncompatible,
            "La biblioteca de paquetes cambió; prepara el motor de nuevo",
        ));
    }
    Ok(())
}
pub fn runtime_version() -> String {
    // alpm_version is stable across majors and returns a static C string.
    unsafe {
        CStr::from_ptr(bindings::alpm_version())
            .to_string_lossy()
            .into_owned()
    }
}
pub fn verify_abi() -> AppResult<()> {
    validate_abi(env!("APP_UPDATES_ALPM_VERSION"), &runtime_version())
}
pub fn arch_compare(a: &str, b: &str) -> AppResult<Ordering> {
    verify_abi()?;
    let a = cstring(a)?;
    let b = cstring(b)?;
    // Valid C strings remain alive for this call; return value is a comparison.
    Ok(unsafe { bindings::alpm_pkg_vercmp(a.as_ptr(), b.as_ptr()) }.cmp(&0))
}
pub(crate) fn cstring(s: &str) -> AppResult<CString> {
    CString::new(s).map_err(|_| AppError::new(ErrorCode::InvalidInput, "Dato con byte nulo"))
}
pub(crate) unsafe fn ctext(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
pub(crate) unsafe fn list_ptrs(list: *mut bindings::alpm_list_t) -> Vec<*mut std::ffi::c_void> {
    let mut result = Vec::new();
    let mut node = list;
    while !node.is_null() {
        result.push(unsafe { (*node).data });
        node = unsafe { (*node).next };
    }
    result
}
