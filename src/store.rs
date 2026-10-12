use crate::{domain::*, paths::Paths};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
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
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            State::default()
        };
        let conf = self.paths.config.join("config.json");
        check_plain(&conf)?;
        if conf.exists() {
            state.config = serde_json::from_slice(&fs::read(conf)?)?;
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
    check_plain(path)?;
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
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
            fs::copy(path, &backup)?;
            fs::set_permissions(backup, fs::Permissions::from_mode(0o600))?;
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
