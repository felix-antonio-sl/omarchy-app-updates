use crate::{alpm::AvailablePackage, domain::*, policy::Policy};
use std::{
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};
pub const HELPER: &str = "/usr/lib/omarchy-app-updates/app-updates-helper";
#[derive(Debug, Clone)]
pub struct Caller {
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}
pub fn validate_caller(
    effective_uid: u32,
    pkexec_uid: Option<&str>,
    passwd: &str,
) -> AppResult<Caller> {
    let invalid = || {
        AppError::new(
            ErrorCode::AuthUnavailable,
            "El ejecutor necesita un operador válido de pkexec",
        )
    };
    if effective_uid != 0 {
        return Err(invalid());
    }
    let uid = pkexec_uid
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|u| *u != 0)
        .ok_or_else(invalid)?;
    for line in passwd.lines() {
        let fields = line.split(':').collect::<Vec<_>>();
        if fields.len() == 7 && fields[2].parse::<u32>().ok() == Some(uid) {
            let gid = fields[3].parse::<u32>().map_err(|_| invalid())?;
            let home = PathBuf::from(fields[5]);
            if !home.is_absolute() || home == Path::new("/") || home == Path::new("/root") {
                return Err(invalid());
            }
            return Ok(Caller { uid, gid, home });
        }
    }
    Err(invalid())
}
pub fn caller() -> AppResult<Caller> {
    trusted_root_path(Path::new("/etc/passwd"))?;
    let passwd = std::fs::read_to_string("/etc/passwd")?;
    validate_caller(
        rustix::process::geteuid().as_raw(),
        std::env::var("PKEXEC_UID").ok().as_deref(),
        &passwd,
    )
}
pub fn trusted_root_path(path: &Path) -> AppResult<()> {
    if !path.is_absolute() {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Ruta administrativa inválida",
        ));
    }
    let mut parent = PathBuf::from("/");
    for c in path.components() {
        match c {
            Component::RootDir => (),
            Component::Normal(p) => {
                parent.push(p);
                let m = std::fs::symlink_metadata(&parent)?;
                if m.file_type().is_symlink() || m.uid() != 0 || m.mode() & 0o022 != 0 {
                    return Err(AppError::new(
                        ErrorCode::UnsafeArtifact,
                        "Ruta administrativa modificable fuera de root",
                    ));
                }
            }
            _ => {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "Ruta administrativa inválida",
                ));
            }
        }
    }
    Ok(())
}
pub fn validate_helper_path(path: &Path) -> AppResult<()> {
    if path != Path::new(HELPER) {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "El ejecutor debe estar instalado en su ruta administrativa",
        ));
    }
    trusted_root_path(path)?;
    let m = std::fs::metadata(path)?;
    if !m.is_file() || m.mode() & 0o111 == 0 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Ejecutor administrativo inválido",
        ));
    }
    Ok(())
}
pub fn validate_admin_config(path: &Path) -> AppResult<()> {
    if path != Path::new("/etc/pacman.conf") {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "No se permite configuración administrativa del usuario",
        ));
    }
    trusted_root_path(path)
}
pub fn validate_resolved_packages(packages: &[AvailablePackage], policy: &Policy) -> AppResult<()> {
    if !policy.coverage.verified() {
        return Err(AppError::new(
            ErrorCode::CoverageNeedsReview,
            "Cambió la cobertura del actualizador de IA",
        ));
    }
    for p in packages {
        if policy.protected_package(&p.name)
            || p.files
                .iter()
                .any(|f| policy.protected_path(&Path::new("/").join(f)))
        {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                format!(
                    "{} toca una herramienta cubierta por updater.py; se detiene toda la transacción",
                    p.name
                ),
            ));
        }
    }
    Ok(())
}
