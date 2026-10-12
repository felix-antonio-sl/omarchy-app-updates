use super::{bindings as ffi, ctext, list_ptrs};
use crate::{
    domain::*,
    execution::EventSink,
    helper::{DecisionValue, HelperQuestion, QuestionId},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    sync::{Arc, Mutex},
};
pub struct CallbackState {
    pub pending: Option<HelperQuestion>,
    pub answers: BTreeMap<QuestionId, DecisionValue>,
    pub asked: BTreeMap<QuestionId, HelperQuestion>,
    pub operation: OperationId,
    pub sink: Arc<dyn EventSink>,
    pub phase: Phase,
    pub seq: u64,
}
impl CallbackState {
    pub fn new(operation: OperationId, sink: Arc<dyn EventSink>) -> Self {
        Self {
            pending: None,
            answers: BTreeMap::new(),
            asked: BTreeMap::new(),
            operation,
            sink,
            phase: Phase::Prepared,
            seq: 0,
        }
    }
    pub fn question(
        &mut self,
        kind: &str,
        message: String,
        choices: Vec<String>,
    ) -> Option<DecisionValue> {
        let id: QuestionId = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(kind, &message, &choices)).unwrap_or_default())
        )
        .into();
        let q = HelperQuestion {
            id: id.clone(),
            kind: kind.into(),
            message,
            choices,
        };
        self.asked.insert(id.clone(), q.clone());
        if let Some(answer) = self.answers.get(&id) {
            Some(answer.clone())
        } else {
            if self.pending.is_none() {
                self.pending = Some(q);
            }
            None
        }
    }
    pub fn emit(&mut self, message: String, progress: Option<f64>) {
        self.seq += 1;
        let _ = self.sink.send(OperationEvent {
            operation_id: self.operation.clone(),
            seq: self.seq,
            phase: self.phase.clone(),
            message,
            progress,
            detail: None,
        });
    }
}
unsafe fn pkg_name(pkg: *mut ffi::alpm_pkg_t) -> String {
    if pkg.is_null() {
        "desconocido".into()
    } else {
        unsafe { ctext(ffi::alpm_pkg_get_name(pkg)) }
    }
}
unsafe fn provider_name(pkg: *mut ffi::alpm_pkg_t) -> String {
    unsafe {
        let db = ffi::alpm_pkg_get_db(pkg);
        let repo = if db.is_null() {
            String::new()
        } else {
            ctext(ffi::alpm_db_get_name(db))
        };
        format!(
            "{repo}/{}={}",
            pkg_name(pkg),
            ctext(ffi::alpm_pkg_get_version(pkg))
        )
    }
}
pub unsafe extern "C" fn question(context: *mut c_void, q: *mut ffi::alpm_question_t) {
    if context.is_null() || q.is_null() {
        return;
    }
    // Default answer is rejection. Provider rejection uses the documented out-of-range index.
    unsafe {
        (*q).any.answer = 0;
    }
    let Ok(mut state) = (unsafe { &*context.cast::<Mutex<CallbackState>>() }).lock() else {
        return;
    };
    unsafe {
        match (*q).type_ {
            ffi::ALPM_QUESTION_SELECT_PROVIDER => {
                (*q).select_provider.use_index = -1;
                let choices = list_ptrs((*q).select_provider.providers)
                    .iter()
                    .map(|p| provider_name((*p).cast()))
                    .collect::<Vec<_>>();
                let dep = ffi::alpm_dep_compute_string((*q).select_provider.depend);
                let message = format!("Proveedor de {}", ctext(dep));
                ffi::free(dep.cast());
                if let Some(DecisionValue::SelectProvider(selected)) =
                    state.question("provider", message, choices.clone())
                    && let Some(index) = choices.iter().position(|p| p == &selected)
                {
                    (*q).select_provider.use_index = index as i32;
                }
            }
            ffi::ALPM_QUESTION_REPLACE_PKG => {
                let message = format!(
                    "Reemplazar {} por {}",
                    pkg_name((*q).replace.oldpkg),
                    provider_name((*q).replace.newpkg)
                );
                if state.question("replace", message, vec!["approve".into(), "reject".into()])
                    == Some(DecisionValue::Approve)
                {
                    (*q).replace.replace = 1;
                }
            }
            ffi::ALPM_QUESTION_CONFLICT_PKG => {
                let conflict = (*q).conflict.conflict;
                let message = if conflict.is_null() {
                    "Conflicto de paquetes".into()
                } else {
                    format!(
                        "{} entra en conflicto con {}; retirar el segundo paquete",
                        pkg_name((*conflict).package1),
                        pkg_name((*conflict).package2)
                    )
                };
                if state.question("conflict", message, vec!["approve".into(), "reject".into()])
                    == Some(DecisionValue::Approve)
                {
                    (*q).conflict.remove = 1;
                }
            }
            ffi::ALPM_QUESTION_IMPORT_KEY => {
                let message = format!(
                    "Importar clave pública: {} · {}",
                    ctext((*q).import_key.fingerprint),
                    ctext((*q).import_key.uid)
                );
                if state.question(
                    "import_key",
                    message,
                    vec!["approve".into(), "reject".into()],
                ) == Some(DecisionValue::Approve)
                {
                    (*q).import_key.import = 1;
                }
            }
            ffi::ALPM_QUESTION_CORRUPTED_PKG => {
                let message = format!(
                    "Descartar el archivo de caché dañado: {}",
                    ctext((*q).corrupted.filepath)
                );
                if state.question(
                    "corrupted_cache",
                    message,
                    vec!["approve".into(), "reject".into()],
                ) == Some(DecisionValue::Approve)
                {
                    (*q).corrupted.remove = 1;
                }
            }
            ffi::ALPM_QUESTION_INSTALL_IGNOREPKG => {
                state.question(
                    "ignore_pkg",
                    format!(
                        "{} está retenido; no se permite ignorar la restricción",
                        pkg_name((*q).install_ignorepkg.pkg)
                    ),
                    vec!["reject".into()],
                );
            }
            ffi::ALPM_QUESTION_REMOVE_PKGS => {
                state.question(
                    "incomplete_scope",
                    "No se permite omitir paquetes con dependencias sin resolver".into(),
                    vec!["reject".into()],
                );
            }
            _ => {
                state.question(
                    "unsupported_question",
                    "La transacción requiere una decisión no cubierta".into(),
                    vec!["reject".into()],
                );
            }
        }
    }
}
pub unsafe extern "C" fn progress(
    context: *mut c_void,
    _: ffi::alpm_progress_t,
    pkg: *const std::ffi::c_char,
    percent: i32,
    _: usize,
    _: usize,
) {
    if context.is_null() {
        return;
    }
    if let Ok(mut state) = (unsafe { &*context.cast::<Mutex<CallbackState>>() }).lock() {
        state.emit(
            format!("Paquete: {}", unsafe { ctext(pkg) }),
            Some(f64::from(percent.clamp(0, 100)) / 100.0),
        );
    }
}
pub unsafe extern "C" fn event(context: *mut c_void, event: *mut ffi::alpm_event_t) {
    if context.is_null() || event.is_null() {
        return;
    }
    if let Ok(mut state) = (unsafe { &*context.cast::<Mutex<CallbackState>>() }).lock() {
        unsafe {
            match (*event).type_ {
                ffi::ALPM_EVENT_SCRIPTLET_INFO => {
                    state.emit(ctext((*event).scriptlet_info.line), None)
                }
                ffi::ALPM_EVENT_HOOK_RUN_START => {
                    state.emit(format!("Hook: {}", ctext((*event).hook_run.desc)), None)
                }
                ffi::ALPM_EVENT_PKG_RETRIEVE_START => {
                    state.emit("Descargando paquetes".into(), None)
                }
                ffi::ALPM_EVENT_PACKAGE_OPERATION_START => {
                    state.emit("Aplicando cambios de paquetes".into(), None)
                }
                _ => (),
            }
        }
    }
}
