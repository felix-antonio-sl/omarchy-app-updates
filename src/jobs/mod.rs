pub mod control;
pub mod launch;
pub mod recovery;
pub mod refresh;
pub mod worker;
use crate::domain::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRecord {
    pub operation: Operation,
    pub plan: UpdatePlan,
    pub requested_plan: UpdatePlan,
    pub report: Report,
    pub executable: std::path::PathBuf,
    pub executable_sha256: String,
    pub created_at: u64,
    pub boot_id: String,
    pub pending: Option<control::DecisionGate>,
    #[serde(default)]
    pub recipes: std::collections::BTreeMap<TargetId, crate::execution::aur::AurRecipe>,
    #[serde(default)]
    pub processes: std::collections::BTreeMap<AppId, Vec<crate::execution::reopen::RunningApp>>,
    #[serde(default)]
    pub current_targets: Vec<TargetId>,
}
pub fn terminal(phase: &Phase) -> bool {
    matches!(phase, Phase::Completed | Phase::Failed | Phase::Cancelled)
}
pub fn valid_id(id: &OperationId) -> AppResult<()> {
    if uuid::Uuid::parse_str(&id.0).is_err() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "ID de trabajo no válido",
        ));
    }
    Ok(())
}
pub fn job_directory(
    id: &OperationId,
    paths: &crate::paths::Paths,
) -> AppResult<std::path::PathBuf> {
    valid_id(id)?;
    Ok(paths.state.join("jobs").join(&id.0))
}
pub fn load_job(id: &OperationId, paths: &crate::paths::Paths) -> AppResult<JobRecord> {
    let path = job_directory(id, paths)?.join("job.json");
    let bytes = crate::store::read_private(&path, 64 * 1024 * 1024)?
        .ok_or_else(|| AppError::new(ErrorCode::IoError, "No existe el recibo del trabajo"))?;
    let job: JobRecord = serde_json::from_slice(&bytes)?;
    if job.operation.id != *id {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "El recibo pertenece a otra operación",
        ));
    }
    Ok(job)
}
pub fn save_job(job: &JobRecord, paths: &crate::paths::Paths) -> AppResult<()> {
    crate::store::atomic_json(
        &job_directory(&job.operation.id, paths)?.join("job.json"),
        job,
    )?;
    crate::store::Store::new(paths.clone()).update(|state| {
        state
            .operations
            .insert(job.operation.id.clone(), job.operation.clone());
        if state
            .report
            .operation
            .as_ref()
            .is_none_or(|o| o.id == job.operation.id)
        {
            state.report.operation = Some(job.operation.clone());
        }
        for result in job.operation.results.iter().filter(|r| {
            r.error.is_none()
                && matches!(
                    r.outcome,
                    Outcome::Updated | Outcome::Current | Outcome::NeedsReopen
                )
        }) {
            let Some(version) = &result.new_version else {
                continue;
            };
            let approved = job
                .report
                .apps
                .iter()
                .flat_map(|a| &a.components)
                .find(|t| t.id == result.target_id);
            for target in state
                .report
                .apps
                .iter_mut()
                .flat_map(|a| &mut a.components)
                .filter(|t| t.id == result.target_id)
            {
                if approved.is_none_or(|a| a.locator != target.locator)
                    || (target.installed != result.old_version
                        && target.installed != result.new_version)
                {
                    continue;
                }
                target.installed = Some(version.clone());
                if let Some(check) = state
                    .report
                    .checks
                    .iter_mut()
                    .find(|c| c.target_id == target.id)
                {
                    check.installed = Some(version.clone());
                    check.state = if version.kind == VersionKind::FlatpakCommit {
                        if check.available.as_ref() == Some(version) {
                            CheckState::Current
                        } else {
                            check.stale = true;
                            CheckState::Unknown
                        }
                    } else {
                        crate::providers::releases::classify(
                            version,
                            check.available.as_ref(),
                            check.published.as_ref(),
                        )?
                    };
                }
            }
        }
        Ok(())
    })
}
