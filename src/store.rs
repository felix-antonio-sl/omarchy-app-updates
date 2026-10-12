use crate::{domain::*, paths::Paths};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
};
pub struct Store {
    pub paths: Paths,
}
impl Store {
    pub fn new(paths: Paths) -> Self {
        Self { paths }
    }
    pub fn load(&self) -> AppResult<State> {
        let path = self.paths.state.join("state.json");
        check_plain(&path)?;
        let mut state: State = if path.exists() {
            serde_json::from_slice(
                &read_private(&path, 64 * 1024 * 1024)?
                    .ok_or_else(|| AppError::new(ErrorCode::IoError, "Estado desapareció"))?,
            )?
        } else {
            State::default()
        };
        let conf = self.paths.config.join("config.json");
        check_plain(&conf)?;
        if conf.exists() {
            state.config =
                serde_json::from_slice(&read_private(&conf, 1024 * 1024)?.ok_or_else(|| {
                    AppError::new(ErrorCode::IoError, "Configuración desapareció")
                })?)?;
        }
        if state.config.schema_version != 1 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Esquema de configuración desconocido",
            ));
        }
        Ok(state)
    }
    pub fn save(&self, state: &State) -> AppResult<()> {
        let _lock = self.lock()?;
        self.save_unlocked(state)
    }
    pub fn update<T>(&self, mutate: impl FnOnce(&mut State) -> AppResult<T>) -> AppResult<T> {
        let _lock = self.lock()?;
        let mut state = self.load()?;
        let result = mutate(&mut state)?;
        self.save_unlocked(&state)?;
        Ok(result)
    }
    fn lock(&self) -> AppResult<std::fs::File> {
        private_dir(&self.paths.state)?;
        let fd = rustix::fs::openat2(
            rustix::fs::CWD,
            self.paths.state.join("store.lock"),
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, "Lock de estado enlazado"))?;
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
                "Lock de estado no privado",
            ));
        }
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .map_err(std::io::Error::from)?;
        Ok(file)
    }
    fn save_unlocked(&self, state: &State) -> AppResult<()> {
        atomic_json(&self.paths.state.join("state.json"), state)?;
        atomic_json(&self.paths.config.join("config.json"), &state.config)
    }
}
pub fn check_plain(path: &Path) -> AppResult<()> {
    if let Ok(meta) = fs::symlink_metadata(path)
        && meta.file_type().is_symlink()
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "No se permiten enlaces en el estado",
        ));
    }
    Ok(())
}
pub fn private_dir(path: &Path) -> AppResult<()> {
    crate::execution::aur::private_directory(path)
}
pub fn atomic_json<T: serde::Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Ruta sin directorio"))?;
    private_dir(parent)?;
    check_plain(path)?;
    let temp = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| -> AppResult<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        f.write_all(&serde_json::to_vec_pretty(value)?)?;
        f.sync_all()?;
        if path.exists() {
            let backup = path.with_file_name(format!(
                "{}.previous",
                path.file_name().unwrap().to_string_lossy()
            ));
            check_plain(&backup)?;
            let previous = read_private(path, 64 * 1024 * 1024)?.ok_or_else(|| {
                AppError::new(ErrorCode::IoError, "El estado cambió durante el respaldo")
            })?;
            let backup_temp = parent.join(format!(".backup-{}", uuid::Uuid::new_v4()));
            let copied = (|| -> AppResult<()> {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&backup_temp)?;
                file.write_all(&previous)?;
                file.sync_all()?;
                fs::rename(&backup_temp, &backup)?;
                Ok(())
            })();
            let _ = fs::remove_file(&backup_temp);
            copied?;
        }
        fs::rename(&temp, path)?;
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn read_private(path: &Path, max: usize) -> AppResult<Option<Vec<u8>>> {
    use std::{io::Read, os::unix::fs::MetadataExt};
    let fd = match rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "Estado enlazado o inaccesible",
            ));
        }
    };
    let file = std::fs::File::from(fd);
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.nlink() != 1
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
        || meta.len() > max as u64
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Estado no privado o demasiado grande",
        ));
    }
    let mut bytes = vec![];
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Estado demasiado grande",
        ));
    }
    Ok(Some(bytes))
}
