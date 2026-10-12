use crate::{domain::*, execution::ExecutionContext, paths::Paths};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BinaryIdentity {
    pub device: u64,
    pub inode: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunningApp {
    pub pid: u32,
    pub exe: PathBuf,
    pub start_ticks: u64,
    pub binary: BinaryIdentity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReopenApproval {
    pub operation_id: OperationId,
    pub app_id: AppId,
    pub processes: Vec<RunningApp>,
}
pub trait ProcessBackend: Send + Sync {
    fn running(&self, executables: &[PathBuf]) -> AppResult<Vec<RunningApp>>;
    fn matches(&self, process: &RunningApp) -> AppResult<bool>;
    fn close(&self, process: &RunningApp) -> AppResult<()>;
    fn launch(&self, app: &App, paths: &Paths) -> AppResult<()>;
}
pub struct LinuxProcesses;
pub fn start_ticks(stat: &str) -> Option<u64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}
fn identity(pid: u32) -> AppResult<Option<RunningApp>> {
    let proc = PathBuf::from(format!("/proc/{pid}"));
    let metadata = match fs::metadata(&proc) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    if metadata.uid() != rustix::process::geteuid().as_raw() {
        return Ok(None);
    }
    let before = fs::read_to_string(proc.join("stat"))
        .ok()
        .and_then(|s| start_ticks(&s));
    let Some(before) = before else {
        return Ok(None);
    };
    let exe = proc.join("exe");
    let file = match fs::metadata(&exe) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    let link = match fs::read_link(&exe) {
        Ok(link) => link,
        Err(_) => return Ok(None),
    };
    let link = PathBuf::from(
        link.to_string_lossy()
            .strip_suffix(" (deleted)")
            .unwrap_or(&link.to_string_lossy()),
    );
    let hash = match crate::providers::telegram::binary_hash(&exe) {
        Ok(hash) => hash,
        Err(_) => return Ok(None),
    };
    if fs::read_to_string(proc.join("stat"))
        .ok()
        .and_then(|s| start_ticks(&s))
        != Some(before)
    {
        return Ok(None);
    }
    Ok(Some(RunningApp {
        pid,
        exe: link,
        start_ticks: before,
        binary: BinaryIdentity {
            device: file.dev(),
            inode: file.ino(),
            sha256: hash,
        },
    }))
}
pub fn executable_paths(app: &App, _: &Paths) -> AppResult<Vec<PathBuf>> {
    let mut paths = vec![];
    let mut packages = vec![];
    for target in &app.components {
        match &target.locator {
            Locator::TelegramLocal { root } => {
                paths.extend([root.join("Telegram"), root.join("Updater")])
            }
            Locator::SteamClient { root, .. } => paths.extend([
                root.join("ubuntu12_32/steam"),
                root.join("ubuntu12_64/steamwebhelper"),
            ]),
            Locator::Repo { package } | Locator::Aur { package, .. } => {
                packages.push(package.clone())
            }
            _ => (),
        }
    }
    if !packages.is_empty() {
        for package in crate::alpm::system_packages()?
            .into_iter()
            .filter(|p| packages.contains(&p.name))
        {
            for file in package.files {
                let path = Path::new("/").join(file);
                if fs::metadata(&path)
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
pub fn running_apps(app: &App, paths: &Paths) -> AppResult<Vec<RunningApp>> {
    LinuxProcesses.running(&executable_paths(app, paths)?)
}
impl ProcessBackend for LinuxProcesses {
    fn running(&self, executables: &[PathBuf]) -> AppResult<Vec<RunningApp>> {
        let wanted = executables
            .iter()
            .filter_map(|p| fs::metadata(p).ok().map(|m| (m.dev(), m.ino())))
            .collect::<Vec<_>>();
        let mut result = vec![];
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if !fs::metadata(entry.path())
                .is_ok_and(|m| m.uid() == rustix::process::geteuid().as_raw())
            {
                continue;
            }
            if !fs::metadata(entry.path().join("exe"))
                .is_ok_and(|m| wanted.contains(&(m.dev(), m.ino())))
            {
                continue;
            }
            if let Some(process) = identity(pid)? {
                result.push(process);
            }
        }
        result.sort_by_key(|p| p.pid);
        Ok(result)
    }
    fn matches(&self, process: &RunningApp) -> AppResult<bool> {
        Ok(identity(process.pid)?.is_some_and(|current| {
            current.pid == process.pid
                && current.start_ticks == process.start_ticks
                && current.binary == process.binary
        }))
    }
    fn close(&self, process: &RunningApp) -> AppResult<()> {
        if process.pid == 0 || process.pid > i32::MAX as u32 {
            return Err(AppError::new(ErrorCode::InvalidInput, "PID no válido"));
        }
        let pid = rustix::process::Pid::from_raw(process.pid as i32)
            .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "PID no válido"))?;
        let fd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::NONBLOCK)
            .map_err(|_| AppError::new(ErrorCode::PlanChanged, "El proceso terminó o cambió"))?;
        if !self.matches(process)? {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "La identidad del proceso cambió; no se cierra",
            ));
        }
        rustix::process::pidfd_send_signal(&fd, rustix::process::Signal::TERM)
            .map_err(std::io::Error::from)?;
        Ok(())
    }
    fn launch(&self, app: &App, paths: &Paths) -> AppResult<()> {
        let desktop = app
            .desktop_ids
            .iter()
            .filter(|id| !id.contains('/') && id.ends_with(".desktop"))
            .flat_map(|id| paths.application_dirs.iter().map(move |d| d.join(id)))
            .find(|p| p.is_file())
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::Unsupported,
                    "No hay un lanzador válido para volver a abrir esta aplicación",
                )
            })?;
        // Detached user service: no inherited output pipe and no worker-owned GUI lifetime.
        let mut child = Command::new("/usr/bin/uwsm")
            .args(["app", "-t", "service", "--", "/usr/bin/gio", "launch"])
            .arg(desktop)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}
pub fn request_reopen(
    app: &App,
    approval: &ReopenApproval,
    ctx: &ExecutionContext,
) -> AppResult<()> {
    if approval.app_id != app.id || approval.operation_id != ctx.operation_id {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "La aprobación pertenece a otra aplicación u operación",
        ));
    }
    let current = ctx.processes.running(&executable_paths(app, &ctx.paths)?)?;
    if current.iter().any(|p| !approval.processes.contains(p))
        || approval
            .processes
            .iter()
            .any(|p| !ctx.processes.matches(p).unwrap_or(false))
    {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Cambió el conjunto de procesos; requiere otra decisión",
        ));
    }
    // Validate the entire set before sending the first signal.
    for process in &approval.processes {
        if !ctx.processes.matches(process)? {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Un PID fue reutilizado",
            ));
        }
    }
    for process in &approval.processes {
        ctx.processes.close(process)?;
    }
    let until = Instant::now() + Duration::from_secs(8);
    while approval
        .processes
        .iter()
        .any(|p| ctx.processes.matches(p).unwrap_or(true))
    {
        if Instant::now() >= until {
            return Err(AppError::new(
                ErrorCode::Busy,
                "La aplicación sigue abierta; ciérrala desde su ventana antes de reabrir",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    ctx.processes.launch(app, &ctx.paths)
}

pub fn needs_reopen(_app: &App, processes: &[RunningApp]) -> AppResult<bool> {
    for process in processes {
        if !LinuxProcesses.matches(process)? {
            continue;
        }
        match fs::metadata(&process.exe) {
            Ok(metadata) => {
                if metadata.dev() != process.binary.device
                    || metadata.ino() != process.binary.inode
                    || crate::providers::telegram::binary_hash(&process.exe)?
                        != process.binary.sha256
                {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}
