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
        Ok(())
    })
}
