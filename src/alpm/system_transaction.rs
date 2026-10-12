use super::{
    AlpmReader, bindings as ffi, cstring, ctext, list_ptrs,
    transaction::{AlpmTransaction, PackageTransaction, checked, configure},
};
use crate::{
    domain::*,
    execution::EventSink,
    helper::{
        administration::{
            SyncPromotion, seal_download, validate_migration_payload, validate_payload_paths,
        },
        system::installed_digest,
    },
    planner::plan_digest,
};
use std::{
    path::{Component, Path},
    ptr,
};
struct Archive {
    name: String,
    version: String,
    filename: String,
    sha256: String,
    signature: String,
    siglevel: u32,
}
impl AlpmTransaction {
    pub(crate) fn prepare_system(&mut self) -> AppResult<()> {
        let Some(root) = &mut self.root else {
            return Ok(());
        };
        root.revalidate(&self.policy)?;
        let live = AlpmReader::open(&root.config, &root.config.dbpath)?;
        let identity = installed_digest(&live.installed()?)?;
        if let Some(old) = &root.initial_local_digest {
            if old != &identity {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "Cambió la base instalada durante la preparación",
                ));
            }
        } else {
            root.initial_local_digest = Some(identity);
        }
        if root.synchronized || self.installing {
            return Ok(());
        }
        let result = unsafe {
            ffi::alpm_db_update(
                self.reader.handle.as_ptr(),
                ffi::alpm_get_syncdbs(self.reader.handle.as_ptr()),
                0,
            )
        };
        self.awaiting_question()?;
        if result < 0 {
            checked(&self.reader, result)?;
        }
        let root = self.root.as_mut().unwrap();
        // Downloaded database inodes may belong to DownloadUser. Replace them with sealed root inodes.
        for repo in &root.config.repos {
            let db = root.database.join("sync").join(format!("{}.db", repo.name));
            seal_download(&db, &db, None)?;
            let sig = db.with_file_name(format!("{}.db.sig", repo.name));
            if sig.exists() {
                seal_download(&sig, &sig, None)?;
            }
        }
        let reader = AlpmReader::open(&root.config, &root.database)?;
        checked(&reader, unsafe {
            ffi::alpm_option_add_cachedir(
                reader.handle.as_ptr(),
                cstring(&root.cache.to_string_lossy())?.as_ptr(),
            )
        })?;
        configure(&reader)?;
        self.reader = reader;
        root.synchronized = true;
        self.register_callbacks()?;
        Ok(())
    }
    pub(crate) fn verify_downloads(&mut self) -> AppResult<()> {
        if self.root.is_none() {
            return Ok(());
        }
        let h = self.reader.handle.as_ptr();
        let mut archives = vec![];
        unsafe {
            for p in list_ptrs(ffi::alpm_trans_get_add(h)) {
                let pkg = p.cast::<ffi::alpm_pkg_t>();
                let db = ffi::alpm_pkg_get_db(pkg);
                // User-built archives were sealed, inspected and signature-checked before add_pkg.
                if db.is_null() {
                    continue;
                }
                archives.push(Archive {
                    name: ctext(ffi::alpm_pkg_get_name(pkg)),
                    version: ctext(ffi::alpm_pkg_get_version(pkg)),
                    filename: ctext(ffi::alpm_pkg_get_filename(pkg)),
                    sha256: ctext(ffi::alpm_pkg_get_sha256sum(pkg)),
                    signature: ctext(ffi::alpm_pkg_get_base64_sig(pkg)),
                    siglevel: ffi::alpm_db_get_siglevel(db) as u32,
                });
            }
        }
        if !self.installing {
            self.callback
                .lock()
                .map_err(|_| {
                    AppError::new(
                        ErrorCode::TransactionFailed,
                        "Canal de progreso no disponible",
                    )
                })?
                .phase = Phase::Downloading;
            native_commit(&self.reader)?;
            self.awaiting_question()?;
        }
        let root = self.root.as_ref().unwrap();
        let cache = root.cache.clone();
        let cachedirs = root.config.cachedirs.clone();
        for archive in archives {
            let file = Path::new(&archive.filename);
            if file.components().count() != 1
                || !matches!(file.components().next(), Some(Component::Normal(_)))
                || archive.sha256.len() != 64
                || !archive.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "Nombre o hash de paquete no verificable",
                ));
            }
            let dest = cache.join(file);
            let source = std::iter::once(&cache)
                .chain(&cachedirs)
                .map(|d| d.join(file))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    AppError::new(
                        ErrorCode::UnsafeArtifact,
                        format!("No se encontró el paquete descargado {}", archive.name),
                    )
                })?;
            seal_download(&source, &dest, Some(&archive.sha256))?;
            let signature = dest.with_file_name(format!("{}.sig", archive.filename));
            if !archive.signature.is_empty() {
                let mut data = ptr::null_mut();
                let mut size = 0usize;
                let result = unsafe {
                    ffi::alpm_decode_signature(
                        cstring(&archive.signature)?.as_ptr(),
                        &mut data,
                        &mut size,
                    )
                };
                if result != 0 || data.is_null() || size > 1024 * 1024 {
                    if !data.is_null() {
                        unsafe { ffi::free(data.cast()) };
                    }
                    return Err(AppError::new(
                        ErrorCode::UnsafeArtifact,
                        "Firma del repositorio no válida",
                    ));
                }
                let bytes = unsafe { std::slice::from_raw_parts(data, size) };
                let result = crate::helper::administration::write_root_data(&signature, bytes);
                unsafe { ffi::free(data.cast()) };
                result?;
            } else {
                let sig = source.with_file_name(format!("{}.sig", archive.filename));
                if sig.exists() {
                    seal_download(&sig, &signature, None)?;
                }
            }
            let mut pkg = ptr::null_mut();
            let result = unsafe {
                ffi::alpm_pkg_load(
                    h,
                    cstring(&dest.to_string_lossy())?.as_ptr(),
                    1,
                    archive.siglevel as i32,
                    &mut pkg,
                )
            };
            if result != 0 {
                self.awaiting_question()?;
                checked(&self.reader, result)?;
            }
            if pkg.is_null() {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "No se pudo inspeccionar el paquete completo",
                ));
            }
            // pkg_load owns the result independently of the active transaction.
            let inspection = (|| -> AppResult<()> {
                unsafe {
                    if ctext(ffi::alpm_pkg_get_name(pkg)) != archive.name
                        || ctext(ffi::alpm_pkg_get_version(pkg)) != archive.version
                    {
                        return Err(AppError::new(
                            ErrorCode::UnsafeArtifact,
                            "Identidad del paquete distinta a la base de repositorio",
                        ));
                    }
                    let list = ffi::alpm_pkg_get_files(pkg);
                    let mut files = vec![];
                    if !list.is_null() && (*list).count > 0 {
                        if (*list).files.is_null() {
                            return Err(AppError::new(
                                ErrorCode::UnsafeArtifact,
                                "Lista de archivos de paquete incompleta",
                            ));
                        }
                        for f in std::slice::from_raw_parts((*list).files, (*list).count) {
                            files.push(ctext(f.name));
                        }
                    }
                    validate_payload_paths(&files, &self.policy)?;
                    validate_migration_payload(&files)?;
                    Ok(())
                }
            })();
            unsafe { ffi::alpm_pkg_free(pkg) };
            inspection?;
        }
        Ok(())
    }
    pub(crate) fn commit_system(
        &mut self,
        digest: &str,
        sink: &dyn EventSink,
    ) -> AppResult<Vec<TargetResult>> {
        let root = self.root.as_ref().ok_or_else(|| {
            AppError::new(
                ErrorCode::AuthUnavailable,
                "Una vista previa no puede instalar paquetes",
            )
        })?;
        let approved = self
            .prepared
            .clone()
            .filter(|p| p.digest == digest && p.digest == plan_digest(p))
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::PlanChanged,
                    "Confirma el último alcance preparado",
                )
            })?;
        if crate::now() >= approved.expires_at {
            return Err(AppError::new(ErrorCode::PlanExpired, "El plan venció"));
        }
        root.revalidate(&self.policy)?;
        let live = AlpmReader::open(&root.config, &root.config.dbpath)?;
        if Some(installed_digest(&live.installed()?)?) != root.initial_local_digest {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Cambió la base instalada; no se inicia la instalación",
            ));
        }
        drop(live);
        let mut promotion = SyncPromotion::new(
            root.database.join("sync"),
            root.config.dbpath.join("sync"),
            root.stage
                .join(format!("sync-backup-{}", uuid::Uuid::new_v4())),
            0,
        )?;
        self.release_active()?;
        self.root.as_ref().unwrap().handoff_database_lock()?;
        let setup = (|| -> AppResult<()> {
            let root = self.root.as_ref().unwrap();
            let reader = AlpmReader::open(&root.config, &root.config.dbpath)?;
            configure(&reader)?;
            // Pin commit to archives inspected during preparation. Normal pacman hooks remain enabled.
            checked(&reader, unsafe {
                ffi::alpm_option_set_cachedirs(reader.handle.as_ptr(), ptr::null_mut())
            })?;
            checked(&reader, unsafe {
                ffi::alpm_option_add_cachedir(
                    reader.handle.as_ptr(),
                    cstring(&root.cache.to_string_lossy())?.as_ptr(),
                )
            })?;
            let locked = unsafe { ffi::alpm_trans_init(reader.handle.as_ptr(), 0) };
            if locked != 0 {
                return Err(AppError::new(
                    ErrorCode::Busy,
                    "Pacman adquirió el bloqueo durante el relevo; no se promueven bases ni se instalan paquetes",
                ));
            }
            self.reader = reader;
            self.installing = true;
            // The actual native lock is now held; revalidation precedes metadata promotion.
            self.prepare_system()?;
            promotion.promote()?;
            self.register_callbacks()?;
            let revised = self.prepare(&approved)?;
            if revised.digest != digest {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "Cambió el alcance antes de instalar; prepara y confirma otro plan",
                ));
            }
            Ok(())
        })();
        if let Err(error) = setup {
            // Restore while the native lock is still held. If preparation failed before
            // `active` was set, the handle still owns its initial empty transaction.
            promotion.restore()?;
            if self.installing && !self.active {
                checked(&self.reader, unsafe {
                    ffi::alpm_trans_release(self.reader.handle.as_ptr())
                })?;
            }
            self.release_active()?;
            return Err(error);
        }
        {
            let mut cb = self.callback.lock().map_err(|_| {
                AppError::new(
                    ErrorCode::TransactionFailed,
                    "Canal de progreso no disponible",
                )
            })?;
            cb.phase = Phase::Installing;
            cb.emit(
                "Instalando el alcance completo confirmado; se conservan los hooks de pacman"
                    .into(),
                None,
            );
        }
        // Once commit starts, sync databases are retained even on failure: installed changes are not rolled back.
        let commit = native_commit(&self.reader).and_then(|_| {
            finalize_reasons(
                &self.reader,
                &approved
                    .steps
                    .iter()
                    .flat_map(|s| s.package_changes.clone())
                    .collect::<Vec<_>>(),
            )
        });
        self.release_active()?;
        let root = self.root.as_ref().unwrap();
        let verifier = AlpmReader::open(&root.config, &root.config.dbpath)?;
        let installed = verifier.installed()?;
        let step = approved
            .steps
            .iter()
            .find(|s| s.kind == StepKind::ArchFull)
            .unwrap();
        let mut results = vec![];
        for target in &step.targets {
            let name = target
                .0
                .strip_prefix("repo:")
                .or_else(|| target.0.strip_prefix("aur:"))
                .unwrap_or(&target.0);
            let change = step.package_changes.iter().find(|p| p.name == name);
            let actual = installed.iter().find(|p| p.name == name);
            let verified = match (change, actual) {
                (Some(p), None) if p.action == PackageAction::Remove => true,
                (Some(p), Some(a)) if p.action != PackageAction::Remove => {
                    p.new_version == a.version
                }
                (None, Some(_)) => true,
                _ => false,
            };
            let error = if let Err(error) = &commit {
                Some(error.clone())
            } else if !verified {
                Some(AppError::new(
                    ErrorCode::TransactionFailed,
                    format!("La versión instalada de {name} no coincide con el plan"),
                ))
            } else {
                None
            };
            results.push(TargetResult {
                target_id: target.clone(),
                outcome: if error.is_none() {
                    if change.is_none() {
                        Outcome::Current
                    } else {
                        Outcome::Updated
                    }
                } else {
                    Outcome::Failed
                },
                old_version: change
                    .and_then(|p| p.old_version.as_deref())
                    .map(Version::arch),
                new_version: actual.map(|p| Version::arch(&p.version)),
                error,
            });
        }
        // Verify dependencies and replacements too, rather than only selected app targets.
        for change in &step.package_changes {
            let actual = installed.iter().find(|p| p.name == change.name);
            let verified = if change.action == PackageAction::Remove {
                actual.is_none()
            } else {
                actual.is_some_and(|p| p.version == change.new_version)
            };
            if !verified || commit.is_err() {
                let mut cb = self.callback.lock().map_err(|_| {
                    AppError::new(
                        ErrorCode::TransactionFailed,
                        "Canal de progreso no disponible",
                    )
                })?;
                cb.phase = Phase::Failed;
                cb.emit(format!("Verificar {}: la operación terminó con cambios conservados y requiere revisión",change.name),None);
                // No success receipt if a dependency or a hook failed, even if the selected app reached its version.
                for result in &mut results {
                    result.outcome = Outcome::Failed;
                    result.error = Some(commit.as_ref().err().cloned().unwrap_or_else(|| {
                        AppError::new(
                            ErrorCode::TransactionFailed,
                            "El alcance completo no pasó la verificación final",
                        )
                    }));
                }
                break;
            }
        }
        if results.is_empty() {
            commit?;
        }
        let mut cb = self.callback.lock().map_err(|_| {
            AppError::new(
                ErrorCode::TransactionFailed,
                "Canal de progreso no disponible",
            )
        })?;
        cb.phase = Phase::Verifying;
        cb.emit(
            "Verificación del alcance instalado terminada".into(),
            Some(1.0),
        );
        let _ = sink;
        Ok(results)
    }
}
fn native_commit(reader: &AlpmReader) -> AppResult<()> {
    let mut data = ptr::null_mut();
    let result = unsafe { ffi::alpm_trans_commit(reader.handle.as_ptr(), &mut data) };
    if !data.is_null() {
        unsafe {
            let code = ffi::alpm_errno(reader.handle.as_ptr());
            for p in list_ptrs(data) {
                if code == ffi::ALPM_ERR_FILE_CONFLICTS {
                    ffi::alpm_fileconflict_free(p.cast());
                } else {
                    ffi::free(p);
                }
            }
            ffi::alpm_list_free(data);
        }
    }
    checked(reader, result)
}

impl AlpmTransaction {
    pub(crate) fn add_recipe_targets(&mut self, plan: &UpdatePlan) -> AppResult<()> {
        if self.root.is_none() {
            if plan
                .steps
                .iter()
                .any(|s| !s.built_artifacts.is_empty() || !s.build_dependencies.is_empty())
            {
                return Err(AppError::new(
                    ErrorCode::AuthUnavailable,
                    "La vista previa no admite artefactos ni dependencias nuevas",
                ));
            }
            return Ok(());
        }
        let h = self.reader.handle.as_ptr();
        let dependencies = plan
            .steps
            .iter()
            .filter(|s| s.kind == StepKind::ArchFull)
            .flat_map(|s| &s.build_dependencies)
            .collect::<std::collections::BTreeSet<_>>();
        if dependencies.len() > 64 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Demasiadas dependencias adicionales",
            ));
        }
        for name in dependencies {
            if !crate::execution::aur::package_name(name) || self.policy.protected_package(name) {
                return Err(AppError::new(
                    ErrorCode::CoverageNeedsReview,
                    "Dependencia adicional fuera del alcance permitido",
                ));
            }
            let mut candidate = ptr::null_mut();
            unsafe {
                for db in list_ptrs(ffi::alpm_get_syncdbs(h)) {
                    let db = db.cast::<ffi::alpm_db_t>();
                    let mut usage = 0;
                    checked(&self.reader, ffi::alpm_db_get_usage(db, &mut usage))?;
                    if usage as u32 & ffi::ALPM_DB_USAGE_INSTALL == 0 {
                        continue;
                    }
                    let p = ffi::alpm_db_get_pkg(db, cstring(name)?.as_ptr());
                    if !p.is_null() {
                        candidate = p;
                        break;
                    }
                }
            }
            if candidate.is_null() {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "La dependencia de build dejó de estar disponible",
                ));
            }
            if unsafe { ffi::alpm_pkg_should_ignore(h, candidate) } != 0 {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "La dependencia de build está retenida por pacman",
                ));
            }
            let included = unsafe {
                list_ptrs(ffi::alpm_trans_get_add(h))
                    .iter()
                    .any(|p| ctext(ffi::alpm_pkg_get_name((*p).cast())) == *name)
            };
            if !included {
                let current = unsafe {
                    ffi::alpm_db_get_pkg(ffi::alpm_get_localdb(h), cstring(name)?.as_ptr())
                };
                if current.is_null() {
                    checked(&self.reader, unsafe { ffi::alpm_add_pkg(h, candidate) })?;
                }
            }
        }
        let artifacts = plan
            .steps
            .iter()
            .filter(|s| s.kind == StepKind::AurBuild)
            .flat_map(|s| &s.built_artifacts)
            .collect::<Vec<_>>();
        if artifacts.len() > 64 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Demasiados artefactos AUR",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for artifact in artifacts {
            if !seen.insert(&artifact.name)
                || !crate::execution::aur::package_name(&artifact.name)
                || self.policy.protected_package(&artifact.name)
            {
                return Err(AppError::new(
                    ErrorCode::CoverageNeedsReview,
                    "Artefacto AUR duplicado o fuera del alcance permitido",
                ));
            }
            let root = self.root.as_mut().unwrap();
            let key = serde_json::to_string(artifact)?;
            if !root.aur_artifacts.contains_key(&key) {
                let path = crate::helper::staging::stage_build_artifact(artifact, &root.caller)?;
                let mut staged = artifact.clone();
                staged.path = path;
                root.aur_artifacts.insert(key.clone(), staged);
            }
            let path = root.aur_artifacts[&key].path.clone();
            let mut pkg = ptr::null_mut();
            let level = unsafe { ffi::alpm_option_get_local_file_siglevel(h) };
            let result = unsafe {
                ffi::alpm_pkg_load(
                    h,
                    cstring(&path.to_string_lossy())?.as_ptr(),
                    1,
                    level,
                    &mut pkg,
                )
            };
            if result != 0 || pkg.is_null() {
                self.awaiting_question()?;
                checked(&self.reader, result)?;
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "No se pudo cargar el artefacto firmado/verificado",
                ));
            }
            let inspected = (|| -> AppResult<()> {
                unsafe {
                    if ctext(ffi::alpm_pkg_get_name(pkg)) != artifact.name
                        || ctext(ffi::alpm_pkg_get_version(pkg)) != artifact.version
                    {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "El nombre o versión del artefacto difiere del plan aprobado",
                        ));
                    }
                    let info = self.reader.archive(&path, level as u32)?;
                    validate_payload_paths(&info.files, &self.policy)?;
                    validate_migration_payload(&info.files)?;
                    Ok(())
                }
            })();
            if let Err(e) = inspected {
                unsafe { ffi::alpm_pkg_free(pkg) };
                return Err(e);
            }
            let result = unsafe { ffi::alpm_add_pkg(h, pkg) };
            if result != 0 {
                unsafe { ffi::alpm_pkg_free(pkg) };
                checked(&self.reader, result)?;
            }
        }
        Ok(())
    }
}

fn finalize_reasons(reader: &AlpmReader, changes: &[PackageChange]) -> AppResult<()> {
    for change in changes {
        if change.old_version.is_none()
            && change.action == PackageAction::Install
            && change.reason == "build_dependency"
        {
            let pkg = unsafe {
                ffi::alpm_db_get_pkg(
                    ffi::alpm_get_localdb(reader.handle.as_ptr()),
                    cstring(&change.name)?.as_ptr(),
                )
            };
            if pkg.is_null() {
                return Err(AppError::new(
                    ErrorCode::TransactionFailed,
                    "No se pudo registrar la dependencia instalada",
                ));
            }
            checked(reader, unsafe {
                ffi::alpm_pkg_set_reason(pkg, ffi::ALPM_PKG_REASON_DEPEND)
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_normal_commit_installs_only_into_an_isolated_fixture_root() {
        use crate::{alpm::AlpmConfig, io::command::CommandRunner};
        use std::process::Command;
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        let db = d.path().join("db");
        let payload = d.path().join("payload");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(db.join("local")).unwrap();
        std::fs::write(db.join("local/ALPM_DB_VERSION"), "9\n").unwrap();
        std::fs::create_dir_all(payload.join("usr/share/apps-fixture")).unwrap();
        std::fs::write(payload.join(".PKGINFO"),"pkgname = apps-fixture\npkgver = 1.0-1\npkgdesc = Isolated fixture\nurl = https://example.org\nbuilddate = 1\npackager = Apps fixture\nsize = 8\narch = x86_64\nlicense = MIT\n").unwrap();
        std::fs::write(payload.join("usr/share/apps-fixture/proof"), b"verified").unwrap();
        let archive = d.path().join("apps-fixture.pkg.tar.gz");
        assert!(
            Command::new("/usr/bin/tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&payload)
                .args([".PKGINFO", "usr"])
                .status()
                .unwrap()
                .success()
        );
        let config = d.path().join("pacman.conf");
        std::fs::write(&config,format!("[options]\nRootDir = {}\nDBPath = {}\nArchitecture = x86_64\nSigLevel = Never\nLocalFileSigLevel = Never\nLogFile = {}\n",root.display(),db.display(),d.path().join("log").display())).unwrap();
        let cfg = AlpmConfig::load(&config, &CommandRunner::default()).unwrap();
        // This fixture method never uses RootContext or any host administrative path.
        assert!(cfg.root.starts_with(d.path()) && cfg.dbpath.starts_with(d.path()));
        let reader = AlpmReader::open(&cfg, &db).unwrap();
        configure(&reader).unwrap();
        let h = reader.handle.as_ptr();
        let mut pkg = ptr::null_mut();
        let mut data = ptr::null_mut();
        unsafe {
            checked(&reader, ffi::alpm_trans_init(h, 0)).unwrap();
            checked(
                &reader,
                ffi::alpm_pkg_load(
                    h,
                    cstring(&archive.to_string_lossy()).unwrap().as_ptr(),
                    1,
                    0,
                    &mut pkg,
                ),
            )
            .unwrap();
            checked(&reader, ffi::alpm_add_pkg(h, pkg)).unwrap();
            checked(&reader, ffi::alpm_trans_prepare(h, &mut data)).unwrap();
        }
        assert!(!root.join("usr/share/apps-fixture/proof").exists());
        native_commit(&reader).unwrap();
        let dependency = PackageChange {
            action: PackageAction::Install,
            name: "apps-fixture".into(),
            old_version: None,
            new_version: "1.0-1".into(),
            repository: None,
            reason: "build_dependency".into(),
            download_bytes: None,
        };
        finalize_reasons(&reader, &[dependency]).unwrap();
        assert!(
            !reader
                .installed()
                .unwrap()
                .iter()
                .find(|p| p.name == "apps-fixture")
                .unwrap()
                .explicit
        );
        unsafe {
            checked(&reader, ffi::alpm_trans_release(h)).unwrap();
        }
        assert_eq!(
            std::fs::read(root.join("usr/share/apps-fixture/proof")).unwrap(),
            b"verified"
        );
        assert!(
            reader
                .installed()
                .unwrap()
                .iter()
                .any(|p| p.name == "apps-fixture" && p.version == "1.0-1")
        );
    }
}
