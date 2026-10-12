use super::*;
use crate::{
    providers::flatpak::{flatpak_query, target_ref, valid_commit},
    store::Store,
};
pub trait FlatpakBackend: Send + Sync {
    fn prepare(&self, target: &Target, commit: &str) -> AppResult<Vec<FlatpakChange>>;
    fn apply(&self, changes: &[FlatpakChange]) -> AppResult<()>;
}
pub fn prepare_flatpak(target: &Target, commit: &str) -> AppResult<Vec<FlatpakChange>> {
    crate::flatpak_native::NativeFlatpak.prepare(target, commit)
}
pub fn run_flatpak(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<Vec<TargetResult>> {
    if step.kind != StepKind::FlatpakUpdate || step.flatpak_changes.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Plan Flatpak sin alcance preparado",
        ));
    }
    if step
        .flatpak_changes
        .iter()
        .any(|c| !valid_commit(&c.commit) || !matches!(c.operation_type, 0 | 1))
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Plan Flatpak no permitido",
        ));
    }
    let report = Store::new(ctx.paths.clone()).load()?.report;
    let mut targets = vec![];
    for id in &step.targets {
        let target = report
            .apps
            .iter()
            .flat_map(|a| &a.components)
            .find(|t| &t.id == id)
            .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "La aplicación Flatpak cambió"))?;
        let (installation, reference) = target_ref(target)?;
        let change = step
            .flatpak_changes
            .iter()
            .find(|c| c.reference == reference && c.installation == installation)
            .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "Referencia fuera del plan"))?;
        targets.push((id, change));
    }
    if targets.is_empty() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "El plan no tiene aplicaciones",
        ));
    }
    ctx.flatpak.apply(&step.flatpak_changes)?;
    let mut commits = std::collections::BTreeMap::new();
    let mut complete = true;
    for change in &step.flatpak_changes {
        let actual = flatpak_query(
            &ctx.provider,
            &change.installation,
            "info",
            Some("--show-commit"),
            None,
            &change.reference,
        )?;
        complete &= actual == change.commit;
        commits.insert(
            (change.installation.clone(), change.reference.clone()),
            actual,
        );
    }
    let mut results = vec![];
    for (id, change) in targets {
        let actual = commits[&(change.installation.clone(), change.reference.clone())].clone();
        let outcome = if !complete {
            Outcome::Failed
        } else if change.previous_commit.as_deref() == Some(actual.as_str()) {
            Outcome::Current
        } else {
            Outcome::Updated
        };
        results.push(TargetResult {
            target_id: id.clone(),
            error: if outcome == Outcome::Failed {
                Some(AppError::new(
                    ErrorCode::TransactionFailed,
                    "El commit instalado no coincide con el plan",
                ))
            } else {
                None
            },
            old_version: change.previous_commit.as_ref().map(|v| Version {
                raw: v.clone(),
                kind: VersionKind::FlatpakCommit,
                product: None,
            }),
            new_version: Some(Version {
                raw: actual,
                kind: VersionKind::FlatpakCommit,
                product: None,
            }),
            outcome,
        });
    }
    Ok(results)
}
