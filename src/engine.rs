use crate::{
    domain::*,
    io::command::{CommandRunner, CommandSpec},
    paths::Paths,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineMetadata {
    pub protocol: u32,
    pub version: String,
    pub source_digest: String,
    pub alpm_compiled: String,
    pub alpm_runtime: String,
}
pub fn metadata() -> EngineMetadata {
    EngineMetadata {
        protocol: 1,
        version: env!("CARGO_PKG_VERSION").into(),
        source_digest: env!("APP_UPDATES_SOURCE_DIGEST").into(),
        alpm_compiled: env!("APP_UPDATES_ALPM_VERSION").into(),
        alpm_runtime: crate::alpm::runtime_version(),
    }
}
pub fn compatible(actual: &EngineMetadata) -> AppResult<()> {
    let expected = metadata();
    if actual.protocol != expected.protocol
        || actual.source_digest != expected.source_digest
        || actual.version != expected.version
    {
        return Err(AppError::new(
            ErrorCode::NeedsEnginePreparation,
            "Prepara el motor y el ejecutor de la misma versión antes de autorizar",
        ));
    }
    crate::alpm::validate_abi(&expected.alpm_compiled, &actual.alpm_runtime).map_err(|_| {
        AppError::new(
            ErrorCode::NeedsEnginePreparation,
            "Cambió la biblioteca de paquetes; prepara de nuevo el motor",
        )
    })?;
    crate::alpm::validate_abi(&actual.alpm_compiled, &actual.alpm_runtime).map_err(|_| {
        AppError::new(
            ErrorCode::NeedsEnginePreparation,
            "El ejecutor no coincide con la biblioteca instalada",
        )
    })
}
pub fn inspect(path: &Path) -> AppResult<EngineMetadata> {
    let result = CommandRunner::default()
        .run(&CommandSpec::new(path).args(["--version".into()]))
        .map_err(|_| {
            AppError::new(
                ErrorCode::NeedsEnginePreparation,
                "El motor no puede cargar la biblioteca actual; vuelve a prepararlo",
            )
        })?;
    if result.code != 0 {
        return Err(AppError::new(
            ErrorCode::NeedsEnginePreparation,
            "Prepara el motor compatible con la biblioteca actual",
        ));
    }
    serde_json::from_str(&result.stdout).map_err(|_| {
        AppError::new(
            ErrorCode::NeedsEnginePreparation,
            "Metadatos del motor no verificables",
        )
    })
}
pub fn verify_helper(paths: &Paths) -> AppResult<()> {
    crate::helper::validate::validate_helper_path(&paths.helper).map_err(|e| {
        if e.code == ErrorCode::IoError {
            AppError::new(
                ErrorCode::NeedsEnginePreparation,
                "Prepara el ejecutor administrativo del plugin",
            )
        } else {
            e
        }
    })?;
    compatible(&inspect(&paths.helper)?)
}
pub fn verify_disk_engine(executable: &Path, paths: &Paths) -> AppResult<()> {
    compatible(&inspect(executable)?)?;
    verify_helper(paths)
}
