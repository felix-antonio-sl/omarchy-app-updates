use crate::{
    domain::*,
    io::command::{CommandRunner, CommandSpec},
    paths::Paths,
    store::Store,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub operation_id: OperationId,
    pub executable: PathBuf,
    pub unit: String,
    pub home: PathBuf,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitState {
    Active,
    Inactive,
    Unknown,
}
pub trait JobBackend: Send + Sync {
    fn launch(&self, spec: &LaunchSpec) -> AppResult<()>;
    fn state(&self, unit: &str) -> AppResult<UnitState>;
}
pub struct SystemdJobs;
impl JobBackend for SystemdJobs {
    fn launch(&self, spec: &LaunchSpec) -> AppResult<()> {
        let command = CommandSpec::new("/usr/bin/systemd-run").args([
            "--user".into(),
            "--collect".into(),
            format!("--unit={}", spec.unit),
            "--property=KillMode=process".into(),
            "--property=TimeoutStopSec=5".into(),
            format!("--setenv=HOME={}", spec.home.display()),
            "--".into(),
            spec.executable.to_string_lossy().into(),
            "--worker".into(),
            spec.operation_id.0.clone(),
        ]);
        let result = CommandRunner::default().run(&command)?;
        if result.code != 0 {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "No se pudo enviar el trabajo a systemd",
            ));
        }
        Ok(())
    }
    fn state(&self, unit: &str) -> AppResult<UnitState> {
        let result =
            CommandRunner::default().run(&CommandSpec::new("/usr/bin/systemctl").args([
                "--user".into(),
                "show".into(),
                "--property=ActiveState".into(),
                "--property=LoadState".into(),
                "--".into(),
                unit.into(),
            ]))?;
        Ok(
            if result.stdout.lines().any(|s| s == "LoadState=not-found") {
                UnitState::Inactive
            } else if result.code != 0 {
                UnitState::Unknown
            } else {
                match result
                    .stdout
                    .lines()
                    .find_map(|s| s.strip_prefix("ActiveState="))
                {
                    Some("active" | "activating") => UnitState::Active,
                    Some("inactive" | "failed") => UnitState::Inactive,
                    _ => UnitState::Unknown,
                }
            },
        )
    }
}
pub fn submit(plan: &UpdatePlan, paths: &Paths) -> AppResult<OperationId> {
    submit_with_backend(
        plan,
        paths,
        &SystemdJobs,
        &std::env::current_exe()?,
        crate::now(),
    )
}
struct SubmitDirectory {
    path: PathBuf,
    retained: bool,
}
impl Drop for SubmitDirectory {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
pub fn submit_with_backend(
    plan: &UpdatePlan,
    paths: &Paths,
    backend: &dyn JobBackend,
    source: &Path,
    now: u64,
) -> AppResult<OperationId> {
    let policy = crate::policy::Policy::for_paths(paths)?;
    let store = Store::new(paths.clone());
    let recipes = crate::jobs::refresh::plan_recipes(plan, paths)?;
    if store
        .load()?
        .operations
        .values()
        .any(|o| !super::terminal(&o.phase))
    {
        return Err(AppError::new(ErrorCode::Busy, "Ya hay un trabajo en curso"));
    }
    let _preparation_lease = super::worker::worker_lock(paths)?;
    let id: OperationId = uuid::Uuid::new_v4().to_string().into();
    let dir = super::job_directory(&id, paths)?;
    crate::execution::aur::private_directory(&dir)?;
    let mut directory = SubmitDirectory {
        path: dir.clone(),
        retained: false,
    };
    let exe = dir.join("engine");
    let source_file = fs::File::from(
        rustix::fs::openat2(
            rustix::fs::CWD,
            source,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, "Motor de origen enlazado"))?,
    );
    if !source_file.metadata()?.is_file() || source_file.metadata()?.len() > 128 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Motor de origen no válido",
        ));
    }
    let mut out = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o700)
        .open(&exe)?;
    use std::io::Read;
    let count = std::io::copy(&mut source_file.take(128 * 1024 * 1024 + 1), &mut out)?;
    if count > 128 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Motor demasiado grande",
        ));
    }
    out.flush()?;
    out.sync_all()?;
    let digest = crate::providers::telegram::binary_hash(&exe)?;
    let unit = format!("omarchy-app-updates-{}.service", id.0);
    let operation = Operation {
        id: id.clone(),
        plan_digest: plan.digest.clone(),
        phase: Phase::Queued,
        unit: Some(unit.clone()),
        events: vec![],
        results: vec![],
    };
    let report = store.update(|state| {
        crate::planner::validate_plan(plan, &state.report, &policy, now)?;
        if state
            .operations
            .values()
            .any(|o| !super::terminal(&o.phase))
        {
            return Err(AppError::new(
                ErrorCode::Busy,
                "Ya hay una instalación o preparación en curso",
            ));
        }
        state.operations.insert(id.clone(), operation.clone());
        state.report.operation = Some(operation.clone());
        Ok(state.report.clone())
    })?;
    directory.retained = true;
    let job = super::JobRecord {
        operation,
        plan: plan.clone(),
        requested_plan: plan.clone(),
        report,
        executable: exe.clone(),
        executable_sha256: digest,
        created_at: now,
        boot_id: fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .into(),
        pending: None,
        recipes,
        processes: Default::default(),
        current_targets: vec![],
    };
    if let Err(error) = crate::store::atomic_json(&dir.join("job.json"), &job) {
        store.update(|s| {
            if let Some(op) = s.operations.get_mut(&id) {
                op.phase = Phase::Failed;
            }
            Ok(())
        })?;
        return Err(error);
    }
    let spec = LaunchSpec {
        operation_id: id.clone(),
        executable: exe,
        unit,
        home: paths.home.clone(),
    };
    if let Err(error) = backend.launch(&spec) {
        let mut failed = job;
        failed.operation.phase = Phase::Failed;
        super::save_job(&failed, paths)?;
        return Err(error);
    }
    Ok(id)
}
