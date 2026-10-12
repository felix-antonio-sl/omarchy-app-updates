use crate::{domain::*, helper::validate::Caller, paths::Paths, policy::Policy};
use rustix::fs::{AtFlags, CWD, Mode, OFlags, ResolveFlags, openat, openat2};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};
fn unsafe_path() -> AppError {
    AppError::new(
        ErrorCode::UnsafeArtifact,
        "Archivo administrativo enlazado, modificable o con propietario inesperado",
    )
}
pub fn read_user_data(path: &Path, uid: u32, max: usize) -> AppResult<Option<Vec<u8>>> {
    let fd = match openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(unsafe_path()),
    };
    let file = File::from(fd);
    let m = file.metadata()?;
    if !m.is_file() || m.uid() != uid || m.nlink() != 1 {
        return Err(unsafe_path());
    }
    if m.len() > max as u64 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Archivo de política demasiado grande",
        ));
    }
    let mut bytes = vec![];
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Archivo de política demasiado grande",
        ));
    }
    Ok(Some(bytes))
}
pub fn policy_for_caller(caller: &Caller, paths: &Paths) -> AppResult<Policy> {
    if paths.home != caller.home {
        return Err(unsafe_path());
    }
    let source = read_user_data(
        &caller.home.join(".local/share/ai-updaters/updater.py"),
        caller.uid,
        1024 * 1024,
    )?;
    let catalog = read_user_data(
        &paths.plugin.join("catalog/managed-exclusions.json"),
        caller.uid,
        128 * 1024,
    )?;
    Policy::from_contents(
        caller.home.clone(),
        source.as_ref().and_then(|s| std::str::from_utf8(s).ok()),
        catalog.as_deref(),
    )
}
fn directory(path: &Path, owner: u32) -> AppResult<File> {
    let fd = openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| unsafe_path())?;
    let file = File::from(fd);
    let m = file.metadata()?;
    if m.uid() != owner || m.mode() & 0o022 != 0 {
        return Err(unsafe_path());
    }
    Ok(file)
}
fn read_regular(dir: &File, name: &str, owner: u32) -> AppResult<Vec<u8>> {
    let fd = openat(
        dir,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| unsafe_path())?;
    let file = File::from(fd);
    let m = file.metadata()?;
    if !m.is_file()
        || m.uid() != owner
        || m.nlink() != 1
        || m.mode() & 0o022 != 0
        || m.len() > 256 * 1024 * 1024
    {
        return Err(unsafe_path());
    }
    let mut bytes = vec![];
    file.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(unsafe_path());
    }
    Ok(bytes)
}
fn atomic_file(dir: &File, name: &str, bytes: &[u8]) -> AppResult<()> {
    let temp = format!(".apps-{}", uuid::Uuid::new_v4());
    let fd = openat(
        dir,
        temp.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH,
    )
    .map_err(|_| unsafe_path())?;
    let mut file = File::from(fd);
    let result = (|| -> AppResult<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        rustix::fs::renameat(dir, temp.as_str(), dir, name).map_err(std::io::Error::from)?;
        dir.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(dir, temp.as_str(), AtFlags::empty());
    }
    result
}
fn database_name(name: &str) -> bool {
    (name.ends_with(".db") || name.ends_with(".db.sig"))
        && name.len() < 220
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
}
pub struct SyncPromotion {
    source: PathBuf,
    source_fd: File,
    destination: File,
    backup: File,
    owner: u32,
    original: BTreeMap<String, Option<Vec<u8>>>,
    promoted: bool,
}
impl SyncPromotion {
    pub fn new(
        source: PathBuf,
        destination: PathBuf,
        backup: PathBuf,
        owner: u32,
    ) -> AppResult<Self> {
        let source_fd = directory(&source, owner)?;
        let destination = directory(&destination, owner)?;
        fs::DirBuilder::new().mode(0o700).create(&backup)?;
        let backup = directory(&backup, owner)?;
        Ok(Self {
            source,
            source_fd,
            destination,
            backup,
            owner,
            original: BTreeMap::new(),
            promoted: false,
        })
    }
    pub fn promote(&mut self) -> AppResult<()> {
        if self.promoted {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Las bases ya fueron promovidas",
            ));
        }
        let mut names = fs::read_dir(&self.source)?
            .map(|e| e.map(|e| e.file_name().into_string().map_err(|_| unsafe_path())))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .collect::<AppResult<Vec<_>>>()?;
        names.retain(|n| database_name(n));
        names.sort();
        if names.iter().all(|n| !n.ends_with(".db")) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "No hay bases verificadas",
            ));
        }
        // Validate every source before changing any destination.
        let mut prepared = BTreeMap::new();
        for name in &names {
            prepared.insert(
                name.clone(),
                Some(read_regular(&self.source_fd, name, self.owner)?),
            );
        }
        // Replacing an unsigned database must not retain a signature for older bytes.
        for name in names.clone().into_iter().filter(|n| n.ends_with(".db")) {
            let signature = format!("{name}.sig");
            if !prepared.contains_key(&signature) {
                prepared.insert(signature.clone(), None);
                names.push(signature);
            }
        }
        for name in &names {
            let old = match openat(
                &self.destination,
                name.as_str(),
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            ) {
                Ok(fd) => {
                    drop(fd);
                    Some(read_regular(&self.destination, name, self.owner)?)
                }
                Err(rustix::io::Errno::NOENT) => None,
                Err(_) => return Err(unsafe_path()),
            };
            if let Some(bytes) = &old {
                atomic_file(&self.backup, name, bytes)?;
            }
            self.original.insert(name.clone(), old);
        }
        self.promoted = true;
        for (name, bytes) in prepared {
            let changed = if let Some(bytes) = bytes {
                atomic_file(&self.destination, &name, &bytes)
            } else {
                match rustix::fs::unlinkat(&self.destination, name.as_str(), AtFlags::empty()) {
                    Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
                    Err(e) => Err(std::io::Error::from(e).into()),
                }
            };
            if let Err(e) = changed {
                let _ = self.restore();
                return Err(e);
            }
        }
        self.destination.sync_all()?;
        Ok(())
    }
    pub fn restore(&mut self) -> AppResult<()> {
        if !self.promoted {
            return Ok(());
        }
        for (name, old) in &self.original {
            if let Some(bytes) = old {
                atomic_file(&self.destination, name, bytes)?;
            } else {
                match rustix::fs::unlinkat(&self.destination, name.as_str(), AtFlags::empty()) {
                    Ok(()) | Err(rustix::io::Errno::NOENT) => (),
                    Err(e) => return Err(std::io::Error::from(e).into()),
                }
            }
        }
        self.destination.sync_all()?;
        self.promoted = false;
        Ok(())
    }
}

pub fn validate_payload_paths(files: &[String], policy: &Policy) -> AppResult<()> {
    use std::path::Component;
    for file in files {
        let path = Path::new(file);
        if path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || policy.protected_path(&Path::new("/").join(path))
        {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                format!("Contenido de paquete fuera del alcance permitido: {file}"),
            ));
        }
    }
    Ok(())
}
pub fn validate_migration_payload(files: &[String]) -> AppResult<()> {
    let migrations = files
        .iter()
        .filter(|f| f.starts_with("usr/share/omarchy/migrations/") && f.ends_with(".sh"))
        .cloned()
        .collect::<Vec<_>>();
    if migrations.is_empty() {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::Unsupported,
        format!(
            "La actualización incluye migraciones Omarchy sin revisión de compatibilidad con updater.py: {}. Requiere revisión antes de instalar todo el alcance.",
            migrations.join(", ")
        ),
    ))
}
/// Seal a file produced by the configured downloader into a root-owned inode.
/// Its parent paths are fixed administrative directories, never caller paths.
pub(crate) fn seal_download(
    source: &Path,
    destination: &Path,
    expected: Option<&str>,
) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::OpenOptionsExt;
    for path in [source, destination] {
        crate::helper::validate::trusted_root_path(path.parent().ok_or_else(unsafe_path)?)?;
    }
    let fd = openat2(
        CWD,
        source,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| unsafe_path())?;
    let mut input = File::from(fd);
    let m = input.metadata()?;
    if !m.is_file() || m.nlink() != 1 || m.mode() & 0o022 != 0 || m.len() > 2 * 1024 * 1024 * 1024 {
        return Err(unsafe_path());
    }
    let parent = destination.parent().ok_or_else(unsafe_path)?;
    let temp = parent.join(format!(".apps-seal-{}", uuid::Uuid::new_v4()));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(&temp)?;
    let result = (|| -> AppResult<()> {
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > 2 * 1024 * 1024 * 1024 {
                return Err(unsafe_path());
            }
            hash.update(&buffer[..n]);
            output.write_all(&buffer[..n])?;
        }
        let hash = format!("{:x}", hash.finalize());
        if expected.is_some_and(|e| e.len() != 64 || e != hash) {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "El paquete descargado no coincide con el hash del repositorio",
            ));
        }
        output.sync_all()?;
        fs::rename(&temp, destination)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub(crate) fn write_root_data(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path.parent().ok_or_else(unsafe_path)?;
    crate::helper::validate::trusted_root_path(parent)?;
    let dir = directory(parent, 0)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(unsafe_path)?;
    atomic_file(&dir, name, bytes)
}
