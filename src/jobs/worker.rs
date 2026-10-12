use super::{JobRecord, control::DecisionGate};
use crate::{
    domain::*,
    execution::{EventSink, reopen::RunningApp},
    helper::{DecisionValue, QuestionId},
    paths::Paths,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct JobHandle {
    pub paths: Paths,
    inner: Arc<Mutex<JobRecord>>,
    pub cancelled: Arc<AtomicBool>,
}
impl JobHandle {
    pub fn load(operation: &OperationId, paths: &Paths) -> AppResult<Self> {
        Ok(Self {
            paths: paths.clone(),
            inner: Arc::new(Mutex::new(super::load_job(operation, paths)?)),
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn record(&self) -> JobRecord {
        self.inner.lock().unwrap().clone()
    }
    pub fn mutate<T>(&self, change: impl FnOnce(&mut JobRecord) -> AppResult<T>) -> AppResult<T> {
        let mut job = self
            .inner
            .lock()
            .map_err(|_| AppError::new(ErrorCode::IoError, "Estado del trabajo cerrado"))?;
        let result = change(&mut job)?;
        super::save_job(&job, &self.paths)?;
        Ok(result)
    }
    pub fn event(
        &self,
        phase: Phase,
        message: impl Into<String>,
        detail: Option<EventDetail>,
    ) -> AppResult<()> {
        self.mutate(|job| {
            job.operation.phase = phase.clone();
            job.operation.events.push(OperationEvent {
                operation_id: job.operation.id.clone(),
                seq: job.operation.events.last().map_or(1, |e| e.seq + 1),
                phase,
                message: message.into(),
                progress: None,
                detail,
            });
            Ok(())
        })
    }
    pub fn gate(
        &self,
        message: &str,
        choices: Vec<DecisionValue>,
        detail: Option<EventDetail>,
    ) -> AppResult<DecisionGate> {
        let gate = self.mutate(|job| {
            let gate = DecisionGate {
                operation_id: job.operation.id.clone(),
                question_id: QuestionId(uuid::Uuid::new_v4().to_string()),
                digest: job.plan.digest.clone(),
                expires_at: crate::now() + 300,
                choices,
            };
            job.pending = Some(gate.clone());
            Ok(gate)
        })?;
        self.event(
            Phase::AwaitingDecision,
            message,
            detail.or_else(|| {
                Some(EventDetail::Question {
                    id: gate.question_id.0.clone(),
                    kind: "confirm".into(),
                    choices: gate
                        .choices
                        .iter()
                        .map(|v| match v {
                            DecisionValue::Approve => "approve".into(),
                            DecisionValue::Reject => "reject".into(),
                            DecisionValue::SelectProvider(p) => p.clone(),
                        })
                        .collect(),
                })
            }),
        )?;
        Ok(gate)
    }
    pub fn cancellation_gate(&self, phase: Phase, message: &str) -> AppResult<()> {
        self.gate(message, vec![DecisionValue::Reject], None)?;
        self.event(phase, message, None)
    }
    pub fn check_cancelled(&self) -> AppResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(AppError::new(
                ErrorCode::AuthCancelled,
                "Preparación cancelada; los resultados ya confirmados se conservan",
            ))
        } else {
            Ok(())
        }
    }
    pub fn results(&self, results: Vec<TargetResult>) -> AppResult<()> {
        self.mutate(|job| {
            for result in results {
                if let Some(old) = job
                    .operation
                    .results
                    .iter_mut()
                    .find(|r| r.target_id == result.target_id)
                {
                    *old = result;
                } else {
                    job.operation.results.push(result);
                }
            }
            Ok(())
        })
    }
    pub fn revise(&self, plan: UpdatePlan) -> AppResult<()> {
        self.mutate(|job| {
            job.operation.plan_digest = plan.digest.clone();
            job.plan = plan.clone();
            Ok(())
        })?;
        self.event(
            Phase::AwaitingDecision,
            "Revisar el alcance preparado antes de instalar",
            Some(EventDetail::PlanRevised(plan)),
        )
    }
    pub fn clear_gate(&self) -> AppResult<()> {
        self.mutate(|job| {
            job.pending = None;
            Ok(())
        })
    }
}
impl EventSink for JobHandle {
    fn send(&self, event: OperationEvent) -> AppResult<()> {
        if event.operation_id != self.record().operation.id {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Progreso de otra operación",
            ));
        }
        self.mutate(|job| {
            if event.phase == Phase::Installing {
                job.pending = None;
            }
            let mut event = event;
            event.seq = job.operation.events.last().map_or(1, |e| e.seq + 1);
            job.operation.phase = event.phase.clone();
            job.operation.events.push(event);
            Ok(())
        })
    }
}

pub fn worker_lock(paths: &Paths) -> AppResult<std::fs::File> {
    crate::store::private_dir(&paths.state)?;
    let fd = rustix::fs::openat2(
        rustix::fs::CWD,
        paths.state.join("worker.lock"),
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CREATE | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, "Lock de trabajo enlazado"))?;
    let file = std::fs::File::from(fd);
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Lock de trabajo no privado",
        ));
    }
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| AppError::new(ErrorCode::Busy, "Hay otro motor de instalación activo"))?;
    Ok(file)
}

pub fn run_worker(operation: &OperationId, paths: &Paths) -> AppResult<()> {
    let _lease = worker_lock(paths)?;
    let job = JobHandle::load(operation, paths)?;
    let saved = job.record();
    if super::terminal(&saved.operation.phase) || saved.operation.phase != Phase::Queued {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Este trabajo no admite reinicio automático",
        ));
    }
    if std::env::current_exe()? != saved.executable
        || crate::providers::telegram::binary_hash(&saved.executable)? != saved.executable_sha256
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "El ejecutable del trabajo cambió",
        ));
    }
    let server = super::control::ControlServer::start(job.clone())?;
    let result = execute(&job, &server);
    match result {
        Ok(()) => {
            job.clear_gate()?;
            job.event(
                Phase::Completed,
                "Trabajo terminado; consulta el resultado de cada componente",
                None,
            )?;
        }
        Err(error) => {
            job.clear_gate()?;
            let saved = job.record();
            let phase = if error.code == ErrorCode::AuthCancelled {
                Phase::Cancelled
            } else {
                Phase::Failed
            };
            let pending = saved
                .requested_plan
                .steps
                .iter()
                .flat_map(|s| &s.targets)
                .filter(|t| !saved.operation.results.iter().any(|r| &r.target_id == *t))
                .map(|t| TargetResult {
                    target_id: t.clone(),
                    outcome: if saved.current_targets.contains(t) {
                        Outcome::Failed
                    } else {
                        Outcome::Skipped
                    },
                    old_version: saved
                        .report
                        .apps
                        .iter()
                        .flat_map(|a| &a.components)
                        .find(|c| &c.id == t)
                        .and_then(|c| c.installed.clone()),
                    new_version: None,
                    error: Some(error.clone()),
                })
                .collect();
            job.results(pending)?;
            job.event(phase, error.message, None)?;
        }
    }
    Ok(())
}
fn approval(
    job: &JobHandle,
    server: &super::control::ControlServer,
    message: &str,
    detail: Option<EventDetail>,
) -> AppResult<()> {
    job.check_cancelled()?;
    let gate = job.gate(
        message,
        vec![DecisionValue::Approve, DecisionValue::Reject],
        detail,
    )?;
    if server.wait(&gate)? != DecisionValue::Approve {
        return Err(AppError::new(ErrorCode::AuthCancelled, "Plan rechazado"));
    }
    job.check_cancelled()
}
fn administration(
    job: &JobHandle,
    server: &super::control::ControlServer,
    plan: UpdatePlan,
) -> AppResult<()> {
    use crate::helper::{HelperClient, HelperEvent};
    job.check_cancelled()?;
    job.clear_gate()?;
    job.event(
        Phase::AwaitingAuth,
        "Autoriza la preparación administrativa en la ventana del sistema",
        None,
    )?;
    job.revise(plan.clone())?;
    let mut helper = HelperClient::start(&job.record().operation.id, &job.paths)?;
    job.cancellation_gate(
        Phase::Downloading,
        "Preparando y descargando el alcance administrativo; puedes cancelar antes de confirmar",
    )?;
    let mut event = helper.prepare(&plan)?;
    loop {
        match event {
            HelperEvent::Prepared { plan, .. } | HelperEvent::RevisedPlan { plan, .. } => {
                job.revise(plan.clone())?;
                if let Err(error) = approval(
                    job,
                    server,
                    "Confirma el alcance administrativo completo; después de confirmar solo se observa la instalación",
                    Some(EventDetail::PlanRevised(plan.clone())),
                ) {
                    let _ = helper.abort();
                    return Err(error);
                }
                job.clear_gate()?;
                job.event(
                    Phase::Installing,
                    "Instalación confirmada; cerrar el panel no detiene la transacción",
                    None,
                )?;
                helper.confirm(&plan.digest)?;
            }
            HelperEvent::Question {
                question, digest, ..
            } => {
                if digest != job.record().plan.digest {
                    return Err(AppError::new(
                        ErrorCode::PlanChanged,
                        "Pregunta de otro alcance",
                    ));
                }
                let choices = question
                    .choices
                    .iter()
                    .map(|c| match c.as_str() {
                        "approve" => DecisionValue::Approve,
                        "reject" => DecisionValue::Reject,
                        _ => DecisionValue::SelectProvider(c.clone()),
                    })
                    .collect();
                let gate = job.gate(
                    &question.message,
                    choices,
                    Some(EventDetail::Question {
                        id: question.id.0.clone(),
                        kind: question.kind.clone(),
                        choices: question.choices.clone(),
                    }),
                )?;
                let decision = match server.wait(&gate) {
                    Ok(d) => d,
                    Err(error) => {
                        let _ = helper.abort();
                        return Err(error);
                    }
                };
                if decision == DecisionValue::Reject {
                    let _ = helper.abort();
                    return Err(AppError::new(
                        ErrorCode::AuthCancelled,
                        "Decisión administrativa rechazada",
                    ));
                }
                helper.answer(&question.id, &decision)?;
            }
            HelperEvent::Progress { data } => {
                job.send(data)?;
            }
            HelperEvent::Result { results, .. } => {
                let failed = results.iter().any(|r| r.outcome == Outcome::Failed);
                job.results(results)?;
                if failed {
                    return Err(AppError::new(
                        ErrorCode::TransactionFailed,
                        "Una parte del alcance no quedó instalada; se conserva el estado real",
                    ));
                }
                crate::engine::verify_disk_engine(&job.record().executable, &job.paths)?;
                return Ok(());
            }
            HelperEvent::Error { error, .. } => return Err(error),
        }
        event = helper.next_event()?;
    }
}
fn execute(job: &JobHandle, server: &super::control::ControlServer) -> AppResult<()> {
    use crate::execution::{self, ExecutionContext};
    let initial = job.record();
    let policy = crate::policy::Policy::for_paths(&job.paths)?;
    crate::planner::validate_plan(&initial.plan, &initial.report, &policy, crate::now())?;
    let ctx = ExecutionContext::new(
        crate::providers::ProviderContext::new(job.paths.clone()),
        initial.operation.id.clone(),
        job,
    )
    .with_report(initial.report.clone())
    .with_cancellation(job.cancelled.clone());
    let mut captured = std::collections::BTreeMap::<AppId, Vec<RunningApp>>::new();
    for app in initial
        .report
        .apps
        .iter()
        .filter(|a| initial.plan.selected.contains(&a.id))
    {
        captured.insert(
            app.id.clone(),
            execution::reopen::running_apps(app, &job.paths)?,
        );
    }
    job.mutate(|j| {
        j.processes = captured.clone();
        Ok(())
    })?;
    for step in initial.plan.steps.clone() {
        job.check_cancelled()?;
        job.mutate(|j| {
            j.current_targets = step.targets.clone();
            Ok(())
        })?;
        match step.kind {
            StepKind::ArchFull => {
                let mut scope = job.record().plan;
                scope.steps = vec![step.clone()];
                scope.created_at = crate::now();
                scope.expires_at = scope.created_at + 900;
                scope.digest = crate::planner::plan_digest(&scope);
                administration(job, server, scope)?;
            }
            StepKind::AurBuild => {
                job.cancellation_gate(
                    Phase::Building,
                    "Compilando la receta revisada como usuario; puedes cancelar la preparación",
                )?;
                let reference = step
                    .aur_recipe
                    .as_ref()
                    .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "Falta la receta AUR"))?;
                let recipe = initial
                    .recipes
                    .values()
                    .find(|r| {
                        r.digest == reference.digest
                            && r.commit == reference.commit
                            && r.package_base == reference.package_base
                    })
                    .ok_or_else(|| {
                        AppError::new(
                            ErrorCode::PlanChanged,
                            "La receta no pertenece al plan revisado",
                        )
                    })?;
                let artifacts = execution::aur::build_aur(recipe, &ctx)?;
                job.check_cancelled()?;
                let mut scope = initial.plan.clone();
                scope.steps = vec![
                    crate::planner::empty_step(StepKind::ArchFull, vec![]),
                    step.clone(),
                ];
                scope = execution::aur::attach_artifacts(&scope, recipe, &artifacts, crate::now())?;
                job.revise(scope.clone())?;
                approval(
                    job,
                    server,
                    "Revisa los artefactos AUR y sus hashes antes de preparar la instalación",
                    Some(EventDetail::PlanRevised(scope.clone())),
                )?;
                administration(job, server, scope)?;
            }
            StepKind::FlatpakUpdate => {
                job.clear_gate()?;
                job.event(
                    Phase::Installing,
                    "Actualizando el alcance Flatpak confirmado",
                    None,
                )?;
                job.results(execution::flatpak::run_flatpak(&step, &ctx)?)?;
            }
            StepKind::TelegramReplace => {
                job.cancellation_gate(Phase::Downloading,"Descargando Telegram desde su publicación oficial; puedes cancelar la preparación")?;
                let stage = execution::telegram::stage_telegram(&step, &ctx)?;
                job.check_cancelled()?;
                loop {
                    job.clear_gate()?;
                    match execution::telegram::activate_telegram(&stage, &ctx) {
                        Ok(result) => {
                            job.results(vec![result])?;
                            break;
                        }
                        Err(error) if error.code == ErrorCode::Busy => {
                            approval(
                                job,
                                server,
                                "Cierra Telegram desde su ventana y confirma para volver a comprobarlo",
                                None,
                            )?;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            StepKind::SteamUpdate => {
                loop {
                    job.clear_gate()?;
                    job.event(Phase::Verifying,"Comprobando y actualizando el cliente oficial Steam; se conservan los juegos",None)?;
                    match execution::steam::run_steam(&step, &ctx) {
                        Ok(result) => {
                            job.results(vec![result])?;
                            break;
                        }
                        Err(e) if e.code == ErrorCode::Busy => {
                            approval(
                                job,
                                server,
                                "Cierra Steam y sus juegos; confirma para volver a comprobarlo",
                                None,
                            )?;
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            StepKind::Reopen => {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Reabrir requiere la aprobación explícita de la aplicación",
                ));
            }
        }
    }
    for (id, processes) in captured {
        if let Some(app) = initial.report.apps.iter().find(|a| a.id == id)
            && execution::reopen::needs_reopen(app, &processes)?
        {
            job.mutate(|j| {
                for r in &mut j.operation.results {
                    if app.components.iter().any(|t| t.id == r.target_id)
                        && r.outcome == Outcome::Updated
                    {
                        r.outcome = Outcome::NeedsReopen;
                    }
                }
                Ok(())
            })?;
            job.event(Phase::Verifying,format!("{} sigue ejecutando la versión anterior; puedes reabrirla cuando termines tu trabajo",app.name),Some(EventDetail::NeedsReopen(id)))?;
        }
    }
    let latest = job.record();
    if latest
        .operation
        .results
        .iter()
        .any(|r| r.outcome == Outcome::Failed)
    {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            "Consulta los componentes que no se actualizaron",
        ));
    }
    Ok(())
}

/// The Reopen protocol action is an explicit approval for exactly this recorded app/process set.
pub fn reopen(operation: &OperationId, app_id: &AppId, paths: &Paths) -> AppResult<()> {
    let _lease = worker_lock(paths)?;
    let job = super::load_job(operation, paths)?;
    if !super::terminal(&job.operation.phase) {
        return Err(AppError::new(
            ErrorCode::Busy,
            "Espera el resultado del trabajo antes de reabrir",
        ));
    }
    if job.boot_id != std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?.trim() {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Los procesos pertenecen a un arranque anterior",
        ));
    }
    let app = job
        .report
        .apps
        .iter()
        .find(|a| &a.id == app_id)
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Aplicación fuera del trabajo"))?;
    if !job.operation.results.iter().any(|r| {
        r.outcome == Outcome::NeedsReopen && app.components.iter().any(|t| t.id == r.target_id)
    }) {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Esta aplicación no tiene una reapertura pendiente",
        ));
    }
    let processes = job.processes.get(app_id).ok_or_else(|| {
        AppError::new(
            ErrorCode::PlanChanged,
            "Falta la identidad de los procesos originales",
        )
    })?;
    let ctx = crate::execution::ExecutionContext::new(
        crate::providers::ProviderContext::new(paths.clone()),
        operation.clone(),
        &crate::helper::SilentSink,
    )
    .with_report(job.report.clone());
    crate::execution::reopen::request_reopen(
        app,
        &crate::execution::reopen::ReopenApproval {
            app_id: app_id.clone(),
            operation_id: operation.clone(),
            processes: processes.clone(),
        },
        &ctx,
    )?;
    let handle = JobHandle::load(operation, paths)?;
    handle.mutate(|j| {
        for result in &mut j.operation.results {
            if result.outcome == Outcome::NeedsReopen
                && app.components.iter().any(|t| t.id == result.target_id)
            {
                result.outcome = Outcome::Updated;
            }
        }
        Ok(())
    })
}
