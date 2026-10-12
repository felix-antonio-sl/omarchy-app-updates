use crate::{domain::*, helper::validate::trusted_root_path};
use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat, openat2};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};
pub struct StagingArea {
    path: PathBuf,
    directory: File,
}
impl StagingArea {
    pub fn new(path: PathBuf, owner: u32) -> AppResult<Self> {
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let fd = openat2(
            CWD,
            &path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, "Directorio staging inválido"))?;
        let directory = File::from(fd);
        let m = directory.metadata()?;
        if m.uid() != owner || m.mode() & 0o077 != 0 {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "Staging debe ser privado y tener su propietario esperado",
            ));
        }
        Ok(Self { path, directory })
    }
    pub fn copy(&self, source: &Path, expected: &str, caller_uid: u32) -> AppResult<PathBuf> {
        let invalid = || {
            AppError::new(
                ErrorCode::UnsafeArtifact,
                "Artefacto enlazado, modificado o sin checksum válido",
            )
        };
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let fd = openat2(
            CWD,
            source,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| invalid())?;
        let mut input = File::from(fd);
        let m = input.metadata()?;
        if !m.is_file()
            || m.uid() != caller_uid
            || m.nlink() != 1
            || m.len() > 2 * 1024 * 1024 * 1024
        {
            return Err(invalid());
        }
        let name = format!("{}.pkg.tar.zst", uuid::Uuid::new_v4());
        let destination = self.path.join(&name);
        let fd = openat(
            &self.directory,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| invalid())?;
        let mut output = File::from(fd);
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 65536];
        let result = (|| -> AppResult<()> {
            loop {
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                bytes += n as u64;
                if bytes > 2 * 1024 * 1024 * 1024 {
                    return Err(invalid());
                }
                digest.update(&buffer[..n]);
                output.write_all(&buffer[..n])?;
            }
            if format!("{:x}", digest.finalize()) != expected.to_ascii_lowercase() {
                return Err(invalid());
            }
            output.sync_all()?;
            self.directory.sync_all()?;
            Ok(())
        })();
        if let Err(e) = result {
            drop(output);
            let _ =
                rustix::fs::unlinkat(&self.directory, name.as_str(), rustix::fs::AtFlags::empty());
            return Err(e);
        }
        Ok(destination)
    }
}
pub fn stage_artifact(path: &Path, expected: &str, caller_uid: u32) -> AppResult<PathBuf> {
    if !rustix::process::geteuid().is_root() {
        return Err(AppError::new(
            ErrorCode::AuthUnavailable,
            "Staging administrativo requiere root",
        ));
    }
    let base = Path::new("/var/cache/omarchy-app-updates");
    match fs::DirBuilder::new().mode(0o700).create(base) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    trusted_root_path(base)?;
    let area = StagingArea::new(base.join(uuid::Uuid::new_v4().to_string()), 0)?;
    area.copy(path, expected, caller_uid)
}

pub fn validate_build_artifact_path(
    path: &Path,
    caller: &crate::helper::validate::Caller,
) -> AppResult<()> {
    let base = caller
        .home
        .join(".local/state/omarchy-app-updates/aur/builds");
    let relative = path.strip_prefix(base).map_err(|_| {
        AppError::new(
            ErrorCode::UnsafeArtifact,
            "El artefacto debe provenir de un build privado del operador",
        )
    })?;
    let parts = relative.components().collect::<Vec<_>>();
    if parts.len() != 3
        || parts
            .iter()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        || parts[1].as_os_str() != "artifacts"
        || uuid::Uuid::parse_str(&parts[0].as_os_str().to_string_lossy()).is_err()
        || !parts[2].as_os_str().to_string_lossy().contains(".pkg.tar.")
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Ruta de artefacto fuera de los outputs permitidos",
        ));
    }
    Ok(())
}

pub fn stage_build_artifact(
    artifact: &crate::domain::BuiltArtifact,
    caller: &crate::helper::validate::Caller,
) -> AppResult<PathBuf> {
    validate_build_artifact_path(&artifact.path, caller)?;
    let signature_path = artifact.path.with_file_name(format!(
        "{}.sig",
        artifact
            .path
            .file_name()
            .ok_or_else(|| AppError::new(ErrorCode::UnsafeArtifact, "Artefacto sin nombre"))?
            .to_string_lossy()
    ));
    let signature =
        crate::helper::administration::read_user_data(&signature_path, caller.uid, 1024 * 1024)?;
    let destination = stage_artifact(&artifact.path, &artifact.sha256, caller.uid)?;
    if let Some(bytes) = signature {
        let destination = destination.with_file_name(format!(
            "{}.sig",
            destination.file_name().unwrap().to_string_lossy()
        ));
        crate::helper::administration::write_root_data(&destination, &bytes)?;
    }
    Ok(destination)
}
