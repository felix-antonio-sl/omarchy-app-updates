use super::{
    AlpmConfig, AlpmReader, bindings as ffi,
    callbacks::{self, CallbackState},
    cstring, ctext, list_ptrs,
};
use crate::{
    domain::*,
    execution::EventSink,
    helper::{DecisionValue, HelperQuestion, QuestionId},
};
use crate::{
    helper::{SilentSink, validate::validate_resolved_packages},
    planner::{plan_digest, repository_digest},
    policy::Policy,
    providers::arch::RepoSnapshot,
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    ptr,
    sync::{Arc, Mutex},
};
pub trait PackageTransaction {
    fn prepare(&mut self, approved: &UpdatePlan) -> AppResult<UpdatePlan>;
    fn commit(&mut self, digest: &str, sink: &dyn EventSink) -> AppResult<Vec<TargetResult>>;
    fn release(&mut self) -> AppResult<()>;
    fn pending_question(&self) -> Option<HelperQuestion> {
        None
    }
    fn answer(&mut self, _: &QuestionId, _: &DecisionValue) -> AppResult<()> {
        Err(AppError::new(
            ErrorCode::InvalidInput,
            "No hay una pregunta activa",
        ))
    }
}
pub fn signature_level(words: &[String], base: u32) -> AppResult<u32> {
    let mut value = base;
    for word in words {
        let (package, database, mode) = if let Some(mode) = word.strip_prefix("Package") {
            (true, false, mode)
        } else if let Some(mode) = word.strip_prefix("Database") {
            (false, true, mode)
        } else {
            (true, true, word.as_str())
        };
        for shift in [0, 10] {
            if shift == 0 && !package || shift == 10 && !database {
                continue;
            }
            let required = 1 << shift;
            let optional = 2 << shift;
            let marginal = 4 << shift;
            let unknown = 8 << shift;
            match mode {
                "Never" => value &= !(required | optional | marginal | unknown),
                "Required" => {
                    value |= required;
                    value &= !optional;
                }
                "Optional" => value |= required | optional,
                "TrustedOnly" => value &= !(marginal | unknown),
                "TrustAll" => value |= marginal | unknown,
                _ => {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "Modo SigLevel no reconocido; no se cambia la política administrativa",
                    ));
                }
            }
        }
    }
    Ok(value)
}
pub(crate) fn checked(reader: &AlpmReader, result: i32) -> AppResult<()> {
    if result == 0 {
        Ok(())
    } else {
        let code = unsafe { ffi::alpm_errno(reader.handle.as_ptr()) };
        Err(AppError::new(ErrorCode::TransactionFailed, unsafe {
            ctext(ffi::alpm_strerror(code))
        }))
    }
}
pub(crate) fn configure(reader: &AlpmReader) -> AppResult<()> {
    let cfg = &reader.config;
    let h = reader.handle.as_ptr();
    if cfg.xfer_command.is_some() {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "XferCommand necesita un adaptador administrativo; no se sustituye su configuración",
        ));
    }
    let default = signature_level(
        &cfg.siglevel,
        ffi::ALPM_SIG_PACKAGE | ffi::ALPM_SIG_DATABASE | ffi::ALPM_SIG_DATABASE_OPTIONAL,
    )?;
    unsafe {
        checked(
            reader,
            ffi::alpm_option_set_default_siglevel(h, default as i32),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_local_file_siglevel(
                h,
                signature_level(
                    &cfg.local_siglevel,
                    ffi::ALPM_SIG_PACKAGE | ffi::ALPM_SIG_PACKAGE_OPTIONAL,
                )? as i32,
            ),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_remote_file_siglevel(
                h,
                signature_level(&cfg.remote_siglevel, default)? as i32,
            ),
        )?;
        checked(
            reader,
            ffi::alpm_option_add_architecture(h, cstring(&cfg.architecture)?.as_ptr()),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_gpgdir(h, cstring(&cfg.gpgdir.to_string_lossy())?.as_ptr()),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_logfile(h, cstring(&cfg.logfile.to_string_lossy())?.as_ptr()),
        )?;
        for d in &cfg.cachedirs {
            checked(
                reader,
                ffi::alpm_option_add_cachedir(h, cstring(&d.to_string_lossy())?.as_ptr()),
            )?;
        }
        for d in &cfg.hookdirs {
            checked(
                reader,
                ffi::alpm_option_add_hookdir(h, cstring(&d.to_string_lossy())?.as_ptr()),
            )?;
        }
        for p in &cfg.ignorepkgs {
            checked(
                reader,
                ffi::alpm_option_add_ignorepkg(h, cstring(p)?.as_ptr()),
            )?;
        }
        for p in &cfg.ignoregroups {
            checked(
                reader,
                ffi::alpm_option_add_ignoregroup(h, cstring(p)?.as_ptr()),
            )?;
        }
        for p in &cfg.noextract {
            checked(
                reader,
                ffi::alpm_option_add_noextract(h, cstring(p)?.as_ptr()),
            )?;
        }
        for p in &cfg.noupgrade {
            checked(
                reader,
                ffi::alpm_option_add_noupgrade(h, cstring(p)?.as_ptr()),
            )?;
        }
        if let Some(user) = &cfg.download_user {
            checked(
                reader,
                ffi::alpm_option_set_sandboxuser(h, cstring(user)?.as_ptr()),
            )?;
        }
        checked(
            reader,
            ffi::alpm_option_set_checkspace(h, i32::from(cfg.checkspace)),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_usesyslog(h, i32::from(cfg.usesyslog)),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_parallel_downloads(h, cfg.parallel_downloads),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_disable_dl_timeout(h, u16::from(cfg.disable_dl_timeout)),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_disable_sandbox_filesystem(
                h,
                u16::from(cfg.disable_sandbox || cfg.disable_sandbox_filesystem),
            ),
        )?;
        checked(
            reader,
            ffi::alpm_option_set_disable_sandbox_syscalls(
                h,
                u16::from(cfg.disable_sandbox || cfg.disable_sandbox_syscalls),
            ),
        )?;
        for repo in &cfg.repos {
            let db = ffi::alpm_register_syncdb(
                h,
                cstring(&repo.name)?.as_ptr(),
                signature_level(&repo.siglevel, default)? as i32,
            );
            if db.is_null() {
                return Err(AppError::new(
                    ErrorCode::TransactionFailed,
                    "No se pudo registrar el repositorio con su política de firma",
                ));
            }
            for url in &repo.servers {
                checked(reader, ffi::alpm_db_add_server(db, cstring(url)?.as_ptr()))?;
            }
            let mut usage = 0;
            for u in &repo.usage {
                usage |= match u.as_str() {
                    "All" => ffi::ALPM_DB_USAGE_ALL,
                    "Sync" => ffi::ALPM_DB_USAGE_SYNC,
                    "Search" => ffi::ALPM_DB_USAGE_SEARCH,
                    "Install" => ffi::ALPM_DB_USAGE_INSTALL,
                    "Upgrade" => ffi::ALPM_DB_USAGE_UPGRADE,
                    _ => {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "Usage de repositorio no reconocido",
                        ));
                    }
                };
            }
            checked(
                reader,
                ffi::alpm_db_set_usage(
                    db,
                    if usage == 0 {
                        ffi::ALPM_DB_USAGE_ALL
                    } else {
                        usage
                    } as i32,
                ),
            )?;
        }
    }
    Ok(())
}
pub struct AlpmTransaction {
    pub(crate) reader: AlpmReader,
    pub(crate) callback: Box<Mutex<CallbackState>>,
    pub(crate) policy: Policy,
    pub(crate) active: bool,
    pub(crate) prepared: Option<UpdatePlan>,
    pub(crate) root: Option<crate::helper::system::RootContext>,
    pub(crate) installing: bool,
}
impl AlpmTransaction {
    pub fn policy(&self) -> Policy {
        self.policy.clone()
    }
    pub fn preview(
        config: AlpmConfig,
        dbpath: PathBuf,
        policy: Policy,
        operation: OperationId,
    ) -> AppResult<Self> {
        let reader = AlpmReader::open(&config, &dbpath)?;
        configure(&reader)?;
        let callback = Box::new(Mutex::new(CallbackState::new(
            operation,
            Arc::new(SilentSink),
        )));
        let transaction = Self {
            reader,
            callback,
            policy,
            active: false,
            prepared: None,
            root: None,
            installing: false,
        };
        transaction.register_callbacks()?;
        Ok(transaction)
    }
    pub fn system(
        caller: crate::helper::validate::Caller,
        operation: OperationId,
        sink: Arc<dyn EventSink>,
    ) -> AppResult<Self> {
        let (root, policy) = crate::helper::system::RootContext::acquire(caller, &operation)?;
        let reader = AlpmReader::open(&root.config, &root.database)?;
        checked(&reader, unsafe {
            ffi::alpm_option_add_cachedir(
                reader.handle.as_ptr(),
                cstring(&root.cache.to_string_lossy())?.as_ptr(),
            )
        })?;
        configure(&reader)?;
        let callback = Box::new(Mutex::new(CallbackState::new(operation, sink)));
        let transaction = Self {
            reader,
            callback,
            policy,
            active: false,
            prepared: None,
            root: Some(root),
            installing: false,
        };
        transaction.register_callbacks()?;
        Ok(transaction)
    }
    pub(crate) fn register_callbacks(&self) -> AppResult<()> {
        let ctx = (&*self.callback as *const Mutex<CallbackState>)
            .cast_mut()
            .cast();
        unsafe {
            checked(
                &self.reader,
                ffi::alpm_option_set_questioncb(
                    self.reader.handle.as_ptr(),
                    Some(callbacks::question),
                    ctx,
                ),
            )?;
            checked(
                &self.reader,
                ffi::alpm_option_set_progresscb(
                    self.reader.handle.as_ptr(),
                    Some(callbacks::progress),
                    ctx,
                ),
            )?;
            checked(
                &self.reader,
                ffi::alpm_option_set_eventcb(
                    self.reader.handle.as_ptr(),
                    Some(callbacks::event),
                    ctx,
                ),
            )?;
        }
        Ok(())
    }
    pub(crate) fn snapshot(&self) -> AppResult<RepoSnapshot> {
        let dbpath = PathBuf::from(unsafe {
            ctext(ffi::alpm_option_get_dbpath(self.reader.handle.as_ptr()))
        });
        let packages = self.reader.available(&dbpath.join("sync"))?;
        let installed = self.reader.installed()?;
        use sha2::{Digest, Sha256};
        Ok(RepoSnapshot {
            repositories: self.reader.config.repos.clone(),
            config_digest: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&self.reader.config)?)
            ),
            packages,
            installed,
            fingerprint: String::new(),
            dbpath,
            created_at: crate::now(),
        })
    }
    pub(crate) fn release_active(&mut self) -> AppResult<()> {
        if self.active {
            self.active = false;
            checked(&self.reader, unsafe {
                ffi::alpm_trans_release(self.reader.handle.as_ptr())
            })?;
        }
        Ok(())
    }
    pub(crate) fn awaiting_question(&mut self) -> AppResult<()> {
        if self.pending_question().is_some() {
            if !self.installing {
                self.release_active()?;
            }
            Err(AppError::new(
                ErrorCode::Busy,
                "Se necesita una decisión explícita",
            ))
        } else {
            Ok(())
        }
    }
}
impl PackageTransaction for AlpmTransaction {
    fn prepare(&mut self, approved: &UpdatePlan) -> AppResult<UpdatePlan> {
        self.release_active()?;
        self.prepared = None;
        if approved
            .steps
            .iter()
            .filter(|s| s.kind == StepKind::ArchFull)
            .count()
            != 1
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "El ejecutor requiere un único paso de actualización completa",
            ));
        }
        self.callback
            .lock()
            .map_err(|_| {
                AppError::new(
                    ErrorCode::TransactionFailed,
                    "Canal de decisiones no disponible",
                )
            })?
            .pending = None;
        self.prepare_system()?;
        let snapshot = self.snapshot()?;
        let selected = approved
            .steps
            .iter()
            .flat_map(|s| &s.package_changes)
            .filter(|p| p.reason == "app")
            .map(|p| p.name.clone())
            .collect::<BTreeSet<_>>();
        // Native preparation handles provider choices. Restrictions are never bypassed.
        for installed in &snapshot.installed {
            if installed.ignored
                && snapshot.preferred(&installed.name).is_some_and(|p| {
                    super::arch_compare(&p.version, &installed.version)
                        .is_ok_and(|o| o == std::cmp::Ordering::Greater)
                })
            {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    format!(
                        "{} está retenido; revisar coherencia antes de actualizar",
                        installed.name
                    ),
                ));
            }
        }
        if !self.installing {
            let flags = if self.root.is_some() {
                ffi::ALPM_TRANS_FLAG_DOWNLOADONLY
            } else {
                ffi::ALPM_TRANS_FLAG_NOLOCK
            };
            checked(&self.reader, unsafe {
                ffi::alpm_trans_init(self.reader.handle.as_ptr(), flags as i32)
            })?;
        }
        self.active = true;
        let result = unsafe { ffi::alpm_sync_sysupgrade(self.reader.handle.as_ptr(), 0) };
        self.awaiting_question()?;
        checked(&self.reader, result)?;
        let mut data = ptr::null_mut();
        let result = unsafe { ffi::alpm_trans_prepare(self.reader.handle.as_ptr(), &mut data) };
        // Details depend on alpm_errno; release each corresponding owned data object.
        if !data.is_null() {
            let code = unsafe { ffi::alpm_errno(self.reader.handle.as_ptr()) };
            unsafe {
                for p in list_ptrs(data) {
                    match code {
                        ffi::ALPM_ERR_UNSATISFIED_DEPS => ffi::alpm_depmissing_free(p.cast()),
                        ffi::ALPM_ERR_CONFLICTING_DEPS => ffi::alpm_conflict_free(p.cast()),
                        _ => ffi::free(p),
                    }
                }
                ffi::alpm_list_free(data);
            }
        }
        self.awaiting_question()?;
        checked(&self.reader, result)?;
        let mut changes = vec![];
        let mut resolved = vec![];
        unsafe {
            for p in list_ptrs(ffi::alpm_trans_get_add(self.reader.handle.as_ptr())) {
                let pkg = p.cast::<ffi::alpm_pkg_t>();
                let name = ctext(ffi::alpm_pkg_get_name(pkg));
                let new_version = ctext(ffi::alpm_pkg_get_version(pkg));
                let old = snapshot.installed.iter().find(|i| i.name == name);
                let db = ffi::alpm_pkg_get_db(pkg);
                let repo = if db.is_null() {
                    None
                } else {
                    Some(ctext(ffi::alpm_db_get_name(db)))
                };
                if let Some(old) = old
                    && super::arch_compare(&new_version, &old.version)? == std::cmp::Ordering::Less
                {
                    return Err(AppError::new(
                        ErrorCode::PlanChanged,
                        "La transacción requeriría una degradación",
                    ));
                }
                let files = old.map(|i| i.files.clone()).unwrap_or_default();
                resolved.push(super::AvailablePackage {
                    name: name.clone(),
                    version: new_version.clone(),
                    repository: repo.clone().unwrap_or_default(),
                    download_bytes: ffi::alpm_pkg_get_size(pkg).max(0) as u64,
                    filename: ctext(ffi::alpm_pkg_get_filename(pkg)),
                    sha256: ctext(ffi::alpm_pkg_get_sha256sum(pkg)),
                    depends: vec![],
                    provides: vec![],
                    files,
                });
                changes.push(PackageChange {
                    action: if old.is_some() {
                        PackageAction::Upgrade
                    } else {
                        PackageAction::Install
                    },
                    name: name.clone(),
                    old_version: old.map(|p| p.version.clone()),
                    new_version,
                    repository: repo,
                    reason: if selected.contains(&name) {
                        "app"
                    } else if old.is_some() {
                        "system"
                    } else {
                        "dependency"
                    }
                    .into(),
                    download_bytes: Some(ffi::alpm_pkg_get_size(pkg).max(0) as u64),
                });
            }
            for p in list_ptrs(ffi::alpm_trans_get_remove(self.reader.handle.as_ptr())) {
                let pkg = p.cast::<ffi::alpm_pkg_t>();
                let name = ctext(ffi::alpm_pkg_get_name(pkg));
                if self.reader.config.holdpkgs.contains(&name) {
                    return Err(AppError::new(
                        ErrorCode::PlanChanged,
                        "La transacción retiraría un paquete HoldPkg",
                    ));
                }
                if self.policy.protected_package(&name)
                    || snapshot
                        .installed
                        .iter()
                        .filter(|i| i.name == name)
                        .flat_map(|i| &i.files)
                        .any(|f| {
                            self.policy
                                .protected_path(&std::path::Path::new("/").join(f))
                        })
                {
                    return Err(AppError::new(
                        ErrorCode::CoverageNeedsReview,
                        "La retirada toca una herramienta excluida",
                    ));
                }
                changes.push(PackageChange {
                    action: PackageAction::Remove,
                    name,
                    old_version: Some(ctext(ffi::alpm_pkg_get_version(pkg))),
                    new_version: String::new(),
                    repository: None,
                    reason: "replacement".into(),
                    download_bytes: Some(0),
                });
            }
        }
        validate_resolved_packages(&resolved, &self.policy)?;
        self.verify_downloads()?;
        self.awaiting_question()?;
        changes.sort_by(|a, b| a.name.cmp(&b.name));
        let mut plan = approved.clone();
        plan.repository_digest = repository_digest(&snapshot);
        if let Some(root) = &mut self.root {
            root.snapshot();
            if root.snapshot_warning {
                let pending = self
                    .callback
                    .lock()
                    .map_err(|_| {
                        AppError::new(
                            ErrorCode::TransactionFailed,
                            "Canal de decisiones no disponible",
                        )
                    })?
                    .question(
                        "snapshot_unavailable",
                        root.snapshot_notices.join("\n"),
                        vec!["approve".into(), "reject".into()],
                    );
                let _ = pending;
            }
            for notice in &root.snapshot_notices {
                if !plan.notices.contains(notice) {
                    plan.notices.push(notice.clone());
                }
            }
        }
        self.awaiting_question()?;
        plan.steps
            .iter_mut()
            .find(|s| s.kind == StepKind::ArchFull)
            .unwrap()
            .package_changes = changes;
        let callback = self.callback.lock().map_err(|_| {
            AppError::new(
                ErrorCode::TransactionFailed,
                "Canal de decisiones no disponible",
            )
        })?;
        plan.decisions = callback
            .answers
            .iter()
            .map(|(id, value)| PlanDecision {
                question_id: id.0.clone(),
                kind: callback
                    .asked
                    .get(id)
                    .map(|q| q.kind.clone())
                    .unwrap_or_default(),
                value: serde_json::to_string(value).unwrap_or_default(),
            })
            .collect();
        drop(callback);
        plan.digest = plan_digest(&plan);
        self.prepared = Some(plan.clone());
        Ok(plan)
    }
    fn commit(&mut self, digest: &str, sink: &dyn EventSink) -> AppResult<Vec<TargetResult>> {
        self.commit_system(digest, sink)
    }
    fn release(&mut self) -> AppResult<()> {
        self.release_active()
    }
    fn pending_question(&self) -> Option<HelperQuestion> {
        self.callback.lock().ok().and_then(|s| s.pending.clone())
    }
    fn answer(&mut self, id: &QuestionId, value: &DecisionValue) -> AppResult<()> {
        let mut state = self.callback.lock().map_err(|_| {
            AppError::new(
                ErrorCode::TransactionFailed,
                "Canal de decisiones no disponible",
            )
        })?;
        let q = state
            .pending
            .as_ref()
            .filter(|q| &q.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Pregunta anterior"))?;
        let accepted = match value {
            DecisionValue::Approve => {
                q.kind != "provider" && q.choices.iter().any(|c| c == "approve")
            }
            DecisionValue::SelectProvider(p) => q.kind == "provider" && q.choices.contains(p),
            DecisionValue::Reject => false,
        };
        if !accepted {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Respuesta no permitida",
            ));
        }
        state.answers.insert(id.clone(), value.clone());
        state.pending = None;
        Ok(())
    }
}
impl Drop for AlpmTransaction {
    fn drop(&mut self) {
        let _ = self.release_active();
    }
}
