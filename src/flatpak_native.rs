//! All libflatpak pointers and callbacks stay in this module and synchronous call.
use crate::{
    domain::*,
    execution::flatpak::FlatpakBackend,
    providers::flatpak::{target_ref, valid_commit},
};
use std::{
    ffi::{CStr, CString, c_void},
    ptr,
};
#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    clippy::all
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/flatpak.rs"));
}
pub struct NativeFlatpak;
struct Object(*mut c_void);
impl Drop for Object {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ffi::g_object_unref(self.0);
            }
        }
    }
}
fn text(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
fn cstr(s: &str) -> AppResult<CString> {
    CString::new(s).map_err(|_| AppError::new(ErrorCode::InvalidInput, "Dato Flatpak inválido"))
}
fn error(p: *mut ffi::GError) -> AppError {
    let message = if p.is_null() {
        "Flatpak no completó la operación".into()
    } else {
        text(unsafe { (*p).message })
    };
    if !p.is_null() {
        unsafe {
            ffi::g_error_free(p);
        }
    }
    AppError::new(ErrorCode::TransactionFailed, message)
}
fn installation(scope: &FlatpakInstall) -> AppResult<Object> {
    let mut err = ptr::null_mut();
    let p = unsafe {
        match scope {
            FlatpakInstall::User => ffi::flatpak_installation_new_user(ptr::null_mut(), &mut err),
            FlatpakInstall::System { name } if name == "default" => {
                ffi::flatpak_installation_new_system(ptr::null_mut(), &mut err)
            }
            FlatpakInstall::System { name } => {
                crate::providers::flatpak::flatpak_scope(scope)?;
                let name = cstr(name)?;
                ffi::flatpak_installation_new_system_with_id(
                    name.as_ptr(),
                    ptr::null_mut(),
                    &mut err,
                )
            }
        }
    };
    if p.is_null() {
        Err(error(err))
    } else {
        Ok(Object(p.cast()))
    }
}
fn previous_commit(install: *mut ffi::FlatpakInstallation, reference: &str) -> Option<String> {
    let parts = reference.split('/').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let kind = if parts[0] == "app" {
        ffi::FLATPAK_REF_KIND_APP
    } else {
        ffi::FLATPAK_REF_KIND_RUNTIME
    };
    let name = cstr(parts[1]).ok()?;
    let arch = cstr(parts[2]).ok()?;
    let branch = cstr(parts[3]).ok()?;
    let mut err = ptr::null_mut();
    let p = unsafe {
        ffi::flatpak_installation_get_installed_ref(
            install,
            kind,
            name.as_ptr(),
            arch.as_ptr(),
            branch.as_ptr(),
            ptr::null_mut(),
            &mut err,
        )
    };
    if p.is_null() {
        if !err.is_null() {
            unsafe {
                ffi::g_error_free(err);
            }
        }
        return None;
    }
    let value = text(unsafe { ffi::flatpak_ref_get_commit(p.cast()) });
    drop(Object(p.cast()));
    if value.is_empty() { None } else { Some(value) }
}
fn metadata(p: *mut ffi::GKeyFile) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    let mut err = ptr::null_mut();
    let raw = unsafe { ffi::g_key_file_to_data(p, &mut len, &mut err) };
    if raw.is_null() {
        if !err.is_null() {
            unsafe {
                ffi::g_error_free(err);
            }
        }
        return String::new();
    }
    let value = text(raw);
    unsafe {
        ffi::g_free(raw.cast());
    }
    value
}
struct Gate {
    scope: FlatpakInstall,
    install: *mut ffi::FlatpakInstallation,
    approved: Option<Vec<FlatpakChange>>,
    observed: Vec<FlatpakChange>,
    reached: bool,
    matched: bool,
}
// Unknown remotes and automatic rebases are outside the reviewed transaction.
unsafe extern "C" fn reject_remote(
    _: *mut ffi::FlatpakTransaction,
    _: i32,
    _: *const std::ffi::c_char,
    _: *const std::ffi::c_char,
    _: *const std::ffi::c_char,
    _: *mut c_void,
) -> i32 {
    0
}
unsafe extern "C" fn preserve_ref(
    _: *mut ffi::FlatpakTransaction,
    _: *const std::ffi::c_char,
    _: *const std::ffi::c_char,
    _: *const std::ffi::c_char,
    _: *const std::ffi::c_char,
    _: *mut *const std::ffi::c_char,
    _: *mut c_void,
) -> i32 {
    0
}
unsafe extern "C" fn ready(txn: *mut ffi::FlatpakTransaction, data: *mut c_void) -> i32 {
    // GObject runs this synchronously while Gate, transaction and installation live.
    let gate = unsafe { &mut *data.cast::<Gate>() };
    gate.reached = true;
    let list = unsafe { ffi::flatpak_transaction_get_operations(txn) };
    let mut node = list;
    let mut changes = vec![];
    while !node.is_null() {
        let op = unsafe { (*node).data.cast::<ffi::FlatpakTransactionOperation>() };
        let reference = text(unsafe { ffi::flatpak_transaction_operation_get_ref(op) });
        changes.push(FlatpakChange {
            installation: gate.scope.clone(),
            previous_commit: previous_commit(gate.install, &reference),
            reference,
            remote: text(unsafe { ffi::flatpak_transaction_operation_get_remote(op) }),
            commit: text(unsafe { ffi::flatpak_transaction_operation_get_commit(op) }),
            operation_type: unsafe { ffi::flatpak_transaction_operation_get_operation_type(op) },
            download_bytes: unsafe { ffi::flatpak_transaction_operation_get_download_size(op) },
            metadata: metadata(unsafe { ffi::flatpak_transaction_operation_get_metadata(op) }),
            old_metadata: metadata(unsafe {
                ffi::flatpak_transaction_operation_get_old_metadata(op)
            }),
        });
        // get_operations transfers list and operation references to the caller.
        unsafe {
            ffi::g_object_unref(op.cast());
            node = (*node).next;
        }
    }
    unsafe {
        ffi::g_list_free(list);
    }
    changes.sort_by(|a, b| a.reference.cmp(&b.reference));
    gate.observed = changes;
    if let Some(approved) = &gate.approved {
        let normalize = |ops: &[FlatpakChange]| {
            let mut ops = ops.to_vec();
            for o in &mut ops {
                o.download_bytes = 0;
            }
            ops.sort_by(|a, b| a.reference.cmp(&b.reference));
            ops
        };
        gate.matched = normalize(approved) == normalize(&gate.observed);
        if gate.matched { 1 } else { 0 }
    } else {
        0
    }
}
fn transact(
    scope: &FlatpakInstall,
    requested: &[(String, String)],
    approved: Option<&[FlatpakChange]>,
) -> AppResult<Vec<FlatpakChange>> {
    let install = installation(scope)?;
    let mut err = ptr::null_mut();
    let txn = unsafe {
        ffi::flatpak_transaction_new_for_installation(install.0.cast(), ptr::null_mut(), &mut err)
    };
    if txn.is_null() {
        return Err(error(err));
    }
    let txn_obj = Object(txn.cast());
    unsafe {
        ffi::flatpak_transaction_set_no_interaction(txn, if approved.is_some() { 0 } else { 1 });
        ffi::flatpak_transaction_set_include_unused_uninstall_ops(txn, 0);
        let remote_cb = std::mem::transmute::<
            unsafe extern "C" fn(
                *mut ffi::FlatpakTransaction,
                i32,
                *const std::ffi::c_char,
                *const std::ffi::c_char,
                *const std::ffi::c_char,
                *mut c_void,
            ) -> i32,
            unsafe extern "C" fn(),
        >(reject_remote);
        ffi::g_signal_connect_data(
            txn.cast(),
            c"add-new-remote".as_ptr(),
            Some(remote_cb),
            ptr::null_mut(),
            None,
            0,
        );
        let rebase_cb = std::mem::transmute::<
            unsafe extern "C" fn(
                *mut ffi::FlatpakTransaction,
                *const std::ffi::c_char,
                *const std::ffi::c_char,
                *const std::ffi::c_char,
                *const std::ffi::c_char,
                *mut *const std::ffi::c_char,
                *mut c_void,
            ) -> i32,
            unsafe extern "C" fn(),
        >(preserve_ref);
        ffi::g_signal_connect_data(
            txn.cast(),
            c"end-of-lifed-with-rebase".as_ptr(),
            Some(rebase_cb),
            ptr::null_mut(),
            None,
            0,
        );
    }
    for (reference, commit) in requested {
        if !valid_commit(commit) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Commit Flatpak inválido",
            ));
        }
        let reference = cstr(reference)?;
        let commit = cstr(commit)?;
        if unsafe {
            ffi::flatpak_transaction_add_update(
                txn,
                reference.as_ptr(),
                ptr::null_mut(),
                commit.as_ptr(),
                &mut err,
            )
        } == 0
        {
            return Err(error(err));
        }
    }
    let mut gate = Gate {
        scope: scope.clone(),
        install: install.0.cast(),
        approved: approved.map(|a| a.to_vec()),
        observed: vec![],
        reached: false,
        matched: false,
    };
    let signal = cstr(if approved.is_some() {
        "ready"
    } else {
        "ready-pre-auth"
    })?;
    unsafe {
        let cb = std::mem::transmute::<
            unsafe extern "C" fn(*mut ffi::FlatpakTransaction, *mut c_void) -> i32,
            unsafe extern "C" fn(),
        >(ready);
        ffi::g_signal_connect_data(
            txn.cast(),
            signal.as_ptr(),
            Some(cb),
            (&mut gate as *mut Gate).cast(),
            None,
            0,
        );
        let ok = ffi::flatpak_transaction_run(txn, ptr::null_mut(), &mut err);
        if approved.is_some() {
            if gate.reached && !gate.matched {
                if !err.is_null() {
                    ffi::g_error_free(err);
                }
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "Cambió la lista de operaciones Flatpak; prepara un plan nuevo",
                ));
            }
            if ok == 0 {
                return Err(error(err));
            }
        } else {
            if !err.is_null() {
                ffi::g_error_free(err);
            }
            if !gate.reached && ok == 0 {
                return Err(AppError::new(
                    ErrorCode::NetworkError,
                    "Flatpak no pudo preparar el alcance",
                ));
            }
        }
    }
    drop(txn_obj);
    drop(install);
    Ok(gate.observed)
}
impl FlatpakBackend for NativeFlatpak {
    fn prepare(&self, target: &Target, commit: &str) -> AppResult<Vec<FlatpakChange>> {
        let (scope, reference) = target_ref(target)?;
        transact(&scope, &[(reference, commit.into())], None)
    }
    fn apply(&self, changes: &[FlatpakChange]) -> AppResult<()> {
        if changes
            .iter()
            .any(|c| !valid_commit(&c.commit) || !matches!(c.operation_type, 0 | 1))
        {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "Operación Flatpak fuera del alcance permitido",
            ));
        }
        let mut scopes = changes
            .iter()
            .map(|c| c.installation.clone())
            .collect::<Vec<_>>();
        scopes.sort();
        scopes.dedup();
        for scope in scopes {
            let group = changes
                .iter()
                .filter(|c| c.installation == scope)
                .cloned()
                .collect::<Vec<_>>();
            let requested = group
                .iter()
                .filter(|c| c.reference.starts_with("app/"))
                .map(|c| (c.reference.clone(), c.commit.clone()))
                .collect::<Vec<_>>();
            if requested.is_empty() {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "El plan Flatpak no tiene aplicación principal",
                ));
            }
            transact(&scope, &requested, Some(&group))?;
        }
        Ok(())
    }
}
