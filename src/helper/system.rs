use crate::{
    alpm::AlpmConfig,
    domain::*,
    helper::{
        administration::policy_for_caller,
        validate::{Caller, trusted_root_path, validate_admin_config},
    },
    io::command::CommandRunner,
    paths::Paths,
    policy::Policy,
};
use rustix::fs::{CWD, FlockOperation, Mode, OFlags, ResolveFlags, flock, openat2};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
fn unsafe_path() -> AppError {
    AppError::new(
        ErrorCode::UnsafeArtifact,
        "Ruta administrativa modificable o enlazada",
    )
}
pub fn trusted_config() -> AppResult<AlpmConfig> {
    validate_admin_config(Path::new("/etc/pacman.conf"))?;
    trusted_root_path(Path::new("/usr/bin/pacman-conf"))?;
    let mut pending = vec![PathBuf::from("/etc/pacman.conf")];
    let mut visited = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        if visited.len() > 100 {
            return Err(unsafe_path());
        }
        trusted_root_path(&path)?;
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err(unsafe_path());
        }
        for line in fs::read_to_string(&path)?.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "AssumeInstalled" {
                    return Err(AppError::new(
                        ErrorCode::Unsupported,
                        "AssumeInstalled necesita revisión de coherencia; no se ignora la configuración",
                    ));
                }
                if key.trim() == "Include" {
                    let include = PathBuf::from(value.trim());
                    if !include.is_absolute() || value.contains(['*', '?', '[']) {
                        return Err(AppError::new(
                            ErrorCode::Unsupported,
                            "Include necesita una ruta absoluta verificable",
                        ));
                    }
                    pending.push(include);
                }
            }
        }
    }
    let cfg = AlpmConfig::load(Path::new("/etc/pacman.conf"), &CommandRunner::default())?;
    if cfg.root != Path::new("/") {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "La instalación administrativa solo gestiona el sistema actual",
        ));
    }
    for dir in std::iter::once(&cfg.dbpath)
        .chain(std::iter::once(&cfg.gpgdir))
        .chain(&cfg.cachedirs)
        .chain(&cfg.hookdirs)
    {
        trusted_root_path(dir)?;
    }
    if let Some(parent) = cfg.logfile.parent() {
        trusted_root_path(parent)?;
    }
    if cfg.logfile.exists() {
        trusted_root_path(&cfg.logfile)?;
    }
    for repo in &cfg.repos {
        if repo.name.is_empty()
            || repo.name.starts_with('.')
            || !repo
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        {
            return Err(unsafe_path());
        }
        for server in &repo.servers {
            if !reqwest::Url::parse(server).is_ok_and(|u| u.scheme() == "https") {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "El repositorio requiere un servidor HTTPS",
                ));
            }
        }
    }
    Ok(cfg)
}
pub fn config_digest(config: &AlpmConfig) -> AppResult<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(config)?)))
}
struct AdministrativeLocks {
    db_path: PathBuf,
    db_lock: File,
    _omarchy: File,
}
impl AdministrativeLocks {
    fn acquire(caller: &Caller, dbpath: &Path, operation: &OperationId) -> AppResult<Self> {
        let runtime = PathBuf::from(format!("/run/user/{}", caller.uid));
        let d = fs::symlink_metadata(&runtime)?;
        if !d.is_dir()
            || d.file_type().is_symlink()
            || d.uid() != caller.uid
            || d.mode() & 0o077 != 0
        {
            return Err(unsafe_path());
        }
        let path = runtime.join("omarchy-update.lock");
        let fd = openat2(
            CWD,
            &path,
            OFlags::RDWR | OFlags::CREATE | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| unsafe_path())?;
        let omarchy = File::from(fd);
        let m = omarchy.metadata()?;
        if !m.is_file()
            || m.nlink() != 1
            || m.mode() & 0o022 != 0
            || !matches!(m.uid(), 0) && m.uid() != caller.uid
        {
            return Err(unsafe_path());
        }
        if m.uid() == 0 && m.len() == 0 {
            rustix::fs::fchown(
                &omarchy,
                Some(rustix::process::Uid::from_raw(caller.uid)),
                Some(rustix::process::Gid::from_raw(caller.gid)),
            )
            .map_err(std::io::Error::from)?;
        }
        flock(&omarchy, FlockOperation::NonBlockingLockExclusive).map_err(|_| {
            AppError::new(
                ErrorCode::Busy,
                "Omarchy ya tiene una actualización en curso",
            )
        })?;
        trusted_root_path(dbpath)?;
        let db_path = dbpath.join("db.lck");
        let db_lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(OFlags::NOFOLLOW.bits() as i32 | OFlags::CLOEXEC.bits() as i32)
            .open(&db_path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    AppError::new(
                        ErrorCode::Busy,
                        "Pacman ya tiene una transacción; no se retira su bloqueo",
                    )
                } else {
                    e.into()
                }
            })?;
        let mut locks = Self {
            db_path,
            db_lock,
            _omarchy: omarchy,
        };
        writeln!(
            locks.db_lock,
            "Apps {} PID {}",
            operation.0,
            std::process::id()
        )?;
        locks.db_lock.sync_all()?;
        Ok(locks)
    }
}
impl AdministrativeLocks {
    fn release_database(&self) -> AppResult<()> {
        let expected = self.db_lock.metadata()?;
        let actual = fs::symlink_metadata(&self.db_path)?;
        if expected.dev() != actual.dev() || expected.ino() != actual.ino() {
            return Err(AppError::new(
                ErrorCode::Busy,
                "Cambió el bloqueo de pacman; no se retira otro bloqueo",
            ));
        }
        fs::remove_file(&self.db_path)?;
        Ok(())
    }
}
impl Drop for AdministrativeLocks {
    fn drop(&mut self) {
        if let (Ok(expected), Ok(actual)) =
            (self.db_lock.metadata(), fs::symlink_metadata(&self.db_path))
            && expected.dev() == actual.dev()
            && expected.ino() == actual.ino()
        {
            let _ = fs::remove_file(&self.db_path);
        }
    }
}
fn ensure_root_directory(path: &Path, mode: u32) -> AppResult<()> {
    match fs::DirBuilder::new().mode(mode).create(path) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    trusted_root_path(path)?;
    if !fs::metadata(path)?.is_dir() {
        return Err(unsafe_path());
    }
    Ok(())
}
fn copy_database_local(source: &Path, destination: &Path) -> AppResult<()> {
    trusted_root_path(source)?;
    ensure_root_directory(destination, 0o755)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let m = fs::symlink_metadata(&path)?;
        if m.file_type().is_symlink() || m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err(unsafe_path());
        }
        if m.is_dir() {
            copy_database_local(&path, &target)?;
        } else if m.is_file() {
            let fd = openat2(
                CWD,
                &path,
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
                ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            )
            .map_err(|_| unsafe_path())?;
            let input = File::from(fd);
            let stat = input.metadata()?;
            if stat.uid() != 0 || !stat.is_file() || stat.len() > 16 * 1024 * 1024 {
                return Err(unsafe_path());
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .custom_flags(OFlags::NOFOLLOW.bits() as i32)
                .open(target)?;
            std::io::copy(&mut input.take(16 * 1024 * 1024 + 1), &mut output)?;
            output.sync_all()?;
        } else {
            return Err(unsafe_path());
        }
    }
    Ok(())
}
pub(crate) struct RootContext {
    pub caller: Caller,
    pub paths: Paths,
    pub config: AlpmConfig,
    pub config_digest: String,
    pub database: PathBuf,
    pub cache: PathBuf,
    pub stage: PathBuf,
    pub synchronized: bool,
    pub initial_local_digest: Option<String>,
    pub snapshot_ready: bool,
    pub snapshot_warning: bool,
    pub snapshot_notices: Vec<String>,
    _locks: AdministrativeLocks,
}
impl RootContext {
    pub fn acquire(caller: Caller, operation: &OperationId) -> AppResult<(Self, Policy)> {
        if !rustix::process::geteuid().is_root() {
            return Err(AppError::new(
                ErrorCode::AuthUnavailable,
                "El ejecutor administrativo requiere autorización gráfica",
            ));
        }
        if operation.0.is_empty()
            || operation.0.len() > 100
            || !operation
                .0
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Operación administrativa inválida",
            ));
        }
        let mut paths = Paths::for_home(caller.home.clone());
        paths.runtime_dir = format!("/run/user/{}", caller.uid).into();
        let policy = policy_for_caller(&caller, &paths)?;
        if !policy.coverage.verified() {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                "Revisar la política de exclusiones antes de instalar",
            ));
        }
        let config = trusted_config()?;
        let config_digest = config_digest(&config)?;
        let locks = AdministrativeLocks::acquire(&caller, &config.dbpath, operation)?;
        let base = Path::new("/var/cache/omarchy-app-updates-databases");
        ensure_root_directory(base, 0o711)?;
        let stage = base.join(uuid::Uuid::new_v4().to_string());
        ensure_root_directory(&stage, 0o755)?;
        let database = stage.join("database");
        ensure_root_directory(&database, 0o755)?;
        ensure_root_directory(&database.join("sync"), 0o755)?;
        copy_database_local(&config.dbpath.join("local"), &database.join("local"))?;
        let cache = stage.join("cache");
        ensure_root_directory(&cache, 0o755)?;
        Ok((
            Self {
                caller,
                paths,
                config,
                config_digest,
                database,
                cache,
                stage,
                synchronized: false,
                initial_local_digest: None,
                snapshot_ready: false,
                snapshot_warning: false,
                snapshot_notices: vec![],
                _locks: locks,
            },
            policy,
        ))
    }
    pub fn revalidate(&self, policy: &Policy) -> AppResult<()> {
        let current = policy_for_caller(&self.caller, &self.paths)?;
        if !current.coverage.verified() || current.digest() != policy.digest() {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                "Cambió la política de updater.py durante el trabajo",
            ));
        }
        if config_digest(&trusted_config()?)? != self.config_digest {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Cambió la configuración administrativa; prepara otro plan",
            ));
        }
        Ok(())
    }
}

pub(crate) fn installed_digest(packages: &[crate::alpm::PackageFacts]) -> AppResult<String> {
    let mut packages = packages.to_vec();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&packages)?)
    ))
}
fn create_snapshots() -> AppResult<Vec<String>> {
    use crate::io::command::CommandSpec;
    trusted_root_path(Path::new("/usr/bin/snapper"))?;
    trusted_root_path(Path::new("/etc/snapper/configs"))?;
    let runner = CommandRunner::default();
    let output = runner.run(
        &CommandSpec::new("/usr/bin/snapper").args(["--csvout".into(), "list-configs".into()]),
    )?;
    if output.code != 0 {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            "Snapper no pudo enumerar sus configuraciones",
        ));
    }
    let names = output
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| line.split(',').next())
        .map(|name| name.trim_matches('"').to_owned())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Snapper no tiene configuraciones; no se creó snapshot",
        ));
    }
    let mut notices = vec![];
    for name in names {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        {
            return Err(unsafe_path());
        }
        trusted_root_path(&Path::new("/etc/snapper/configs").join(&name))?;
        let result = runner.run(&CommandSpec::new("/usr/bin/snapper").args([
            "-c".into(),
            name.clone(),
            "create".into(),
            "--print-number".into(),
            "--cleanup-algorithm".into(),
            "number".into(),
            "--description".into(),
            "Apps: antes de actualizar paquetes".into(),
        ]))?;
        let id = result.stdout.trim();
        if result.code != 0 || id.is_empty() || !id.bytes().all(|c| c.is_ascii_digit()) {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                format!(
                    "No se pudo crear/verificar snapshot {name}; los snapshots previos de esta preparación se conservan"
                ),
            ));
        }
        notices.push(format!("Snapshot {name} #{id}. Su restauración requiere intervención; no hay reversión automática."));
    }
    Ok(notices)
}
impl RootContext {
    pub fn handoff_database_lock(&self) -> AppResult<()> {
        self._locks.release_database()
    }
    pub fn snapshot(&mut self) {
        if self.snapshot_ready {
            return;
        }
        self.snapshot_ready = true;
        match create_snapshots() {
            Ok(notices) => self.snapshot_notices = notices,
            Err(error) => {
                self.snapshot_warning = true;
                self.snapshot_notices = vec![format!("Sin snapshot confirmado: {}", error.message)];
            }
        }
    }
}
