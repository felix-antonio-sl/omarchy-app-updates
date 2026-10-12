use super::launch::{JobBackend, SystemdJobs, UnitState};
use crate::{domain::*, paths::Paths, store::Store};
pub trait RecoveryVerifier {
    fn installed(&self, target: &Target, paths: &Paths) -> AppResult<Option<Version>>;
}
pub struct NativeRecovery;
impl RecoveryVerifier for NativeRecovery {
    fn installed(&self, target: &Target, paths: &Paths) -> AppResult<Option<Version>> {
        match &target.locator {
            Locator::Repo { package } | Locator::Aur { package, .. } => {
                Ok(crate::alpm::system_packages()?
                    .iter()
                    .find(|p| &p.name == package)
                    .map(|p| Version::arch(&p.version)))
            }
            Locator::Flatpak { .. } => {
                let (install, reference) = crate::providers::flatpak::target_ref(target)?;
                let ctx = crate::providers::ProviderContext::new(paths.clone());
                let actual = crate::providers::flatpak::flatpak_query(
                    &ctx,
                    &install,
                    "info",
                    Some("--show-commit"),
                    None,
                    &reference,
                )?;
                if !crate::providers::flatpak::valid_commit(&actual) {
                    return Err(AppError::new(
                        ErrorCode::TransactionFailed,
                        "Commit desplegado no verificable",
                    ));
                }
                Ok(Some(Version {
                    raw: actual,
                    kind: VersionKind::FlatpakCommit,
                    product: None,
                }))
            }
            Locator::TelegramLocal { root } => crate::providers::telegram::installed_version(root),
            Locator::SteamClient { root, channel } => {
                Ok(Some(crate::providers::steam::manifest_version(
                    &std::fs::read(root.join("package/steam_client_ubuntu12.manifest"))?,
                    channel,
                )?))
            }
            _ => Ok(None),
        }
    }
}
pub fn recover(paths: &Paths) -> AppResult<Vec<Operation>> {
    recover_with_verifier(paths, &SystemdJobs, &NativeRecovery, crate::now())
}
pub fn recover_with_backend(
    paths: &Paths,
    backend: &dyn JobBackend,
    now: u64,
) -> AppResult<Vec<Operation>> {
    recover_with_verifier(paths, backend, &NativeRecovery, now)
}
pub fn recover_with_verifier(
    paths: &Paths,
    backend: &dyn JobBackend,
    verifier: &dyn RecoveryVerifier,
    now: u64,
) -> AppResult<Vec<Operation>> {
    let store = Store::new(paths.clone());
    let state = store.load()?;
    if !state
        .operations
        .values()
        .any(|o| !super::terminal(&o.phase))
    {
        return Ok(state.operations.into_values().collect());
    }
    let _lease = match super::worker::worker_lock(paths) {
        Ok(l) => l,
        Err(e) if e.code == ErrorCode::Busy => return Ok(state.operations.into_values().collect()),
        Err(e) => return Err(e),
    };
    for operation in state
        .operations
        .values()
        .filter(|o| !super::terminal(&o.phase))
    {
        let Some(unit) = &operation.unit else {
            continue;
        };
        let status = backend.state(unit)?;
        // D-Bus failure is not proof that the worker died.
        if status != UnitState::Inactive {
            continue;
        }
        let mut job = match super::load_job(&operation.id, paths) {
            Ok(j) => j,
            Err(_) => {
                store.update(|s| {
                    if let Some(op)=s.operations.get_mut(&operation.id){
                        op.phase=Phase::Failed;
                        op.events.push(OperationEvent{operation_id:op.id.clone(),seq:op.events.last().map_or(1,|e|e.seq+1),phase:Phase::Failed,message:"No hay unidad activa ni recibo legible; comprueba el inventario y prepara un trabajo nuevo".into(),progress:None,detail:None});
                    }
                    if s.report.operation.as_ref().is_some_and(|o|o.id==operation.id){s.report.operation=s.operations.get(&operation.id).cloned();}
                    Ok(())
                })?;
                continue;
            }
        };
        if now.saturating_sub(job.created_at) < 30 {
            continue;
        }
        let error = AppError::new(
            ErrorCode::TransactionFailed,
            "El trabajo fue interrumpido; la versión observada no prueba que se completara toda la transacción",
        );
        for target in job
            .report
            .apps
            .iter()
            .flat_map(|a| &a.components)
            .filter(|t| {
                job.requested_plan
                    .steps
                    .iter()
                    .any(|s| s.targets.contains(&t.id))
            })
        {
            if job
                .operation
                .results
                .iter()
                .any(|r| r.target_id == target.id)
            {
                continue;
            }
            let actual = verifier.installed(target, paths);
            job.operation.results.push(TargetResult {
                target_id: target.id.clone(),
                outcome: Outcome::Failed,
                old_version: target.installed.clone(),
                new_version: actual.as_ref().ok().cloned().flatten(),
                error: Some(actual.err().unwrap_or_else(|| error.clone())),
            });
        }
        job.operation.phase = Phase::Failed;
        job.pending = None;
        job.operation.events.push(OperationEvent{operation_id:operation.id.clone(),seq:job.operation.events.last().map_or(1,|e|e.seq+1),phase:Phase::Failed,message:"La unidad terminó sin un recibo final. Se cotejaron las versiones locales; prepara un plan nuevo. Los cambios ya instalados se conservan.".into(),progress:None,detail:None});
        super::save_job(&job, paths)?;
    }
    Ok(store.load()?.operations.into_values().collect())
}
