use crate::{
    domain::*,
    execution::ExecutionContext,
    io::command::CommandSpec,
    providers::{
        steam::{detect_channel, manifest_url, manifest_version},
        telegram::binary_hash,
    },
};
use std::{
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamState {
    Closed,
    ClientRunning,
    GamesRunning,
    Unknown,
}
pub trait SteamBackend: Send + Sync {
    fn state(&self, root: &Path, ctx: &ExecutionContext) -> AppResult<SteamState>;
    fn start(&self, root: &Path, unit: &str, ctx: &ExecutionContext) -> AppResult<()>;
    fn wait(
        &self,
        root: &Path,
        channel: &str,
        expected: &str,
        before: &str,
        ctx: &ExecutionContext,
    ) -> AppResult<()>;
}
pub struct NativeSteam;
fn manifest(root: &Path, channel: &str) -> AppResult<Version> {
    manifest_version(
        &fs::read(root.join("package/steam_client_ubuntu12.manifest"))?,
        channel,
    )
}
fn installation_hash(root: &Path) -> AppResult<String> {
    binary_hash(&root.join("package/steam_client_ubuntu12.installed"))
}
/// Correlate public installed-file CRC/size records with core executable bytes.
/// CRC is local integrity metadata, not a publisher signature.
pub fn verify_client_installation(
    root: &Path,
    channel: &str,
    expected: &str,
    before: Option<&str>,
) -> AppResult<Version> {
    if detect_channel(root)? != channel {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Steam cambió de canal durante la actualización",
        ));
    }
    let version = manifest(root, channel)?;
    if version.raw != expected {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            "El manifiesto Steam no confirma el build solicitado",
        ));
    }
    let installed = root.join("package/steam_client_ubuntu12.installed");
    let metadata = fs::symlink_metadata(&installed)?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Inventario instalado Steam no válido",
        ));
    }
    let hash = installation_hash(root)?;
    if before.is_some_and(|old| old == hash) {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            "El inventario instalado no cambió; un manifiesto descargado no confirma instalación",
        ));
    }
    let data = fs::read_to_string(installed)?;
    let mut confirmed = 0;
    for required in [
        "ubuntu12_32/steam",
        "ubuntu12_32/steamclient.so",
        "linux64/steamclient.so",
    ] {
        let row = data.lines().find_map(|line| {
            line.split_once(',')
                .filter(|(name, _)| *name == required)
                .map(|(_, fields)| fields)
        });
        let Some(row) = row else {
            if required == "ubuntu12_32/steam" {
                return Err(AppError::new(
                    ErrorCode::TransactionFailed,
                    "El inventario no confirma el ejecutable Steam",
                ));
            }
            continue;
        };
        let fields = row.split(';').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "Registro Steam incompleto",
            ));
        }
        let size = fields[0]
            .parse::<u64>()
            .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, "Tamaño Steam no válido"))?;
        let crc = fields[2]
            .parse::<u32>()
            .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, "CRC Steam no válido"))?;
        let path = root.join(required);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != size
            || size > 256 * 1024 * 1024
        {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "El binario Steam no coincide con su inventario",
            ));
        }
        let mut file = fs::File::open(path)?;
        let mut hasher = crc32fast::Hasher::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        if hasher.finalize() != crc {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "El checksum de un binario Steam no coincide con la instalación",
            ));
        }
        confirmed += 1;
    }
    if confirmed == 0 {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            "No se confirmó ningún binario cliente",
        ));
    }
    Ok(version)
}
impl SteamBackend for NativeSteam {
    fn state(&self, root: &Path, ctx: &ExecutionContext) -> AppResult<SteamState> {
        let mut unknown = false;
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if !entry
                .file_name()
                .to_str()
                .is_some_and(|s| s.bytes().all(|c| c.is_ascii_digit()))
                || !fs::metadata(entry.path())
                    .is_ok_and(|m| m.uid() == rustix::process::geteuid().as_raw())
            {
                continue;
            }
            let file = match fs::File::open(entry.path().join("environ")) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    unknown = true;
                    continue;
                }
            };
            let mut bytes = vec![];
            file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 256 * 1024 {
                unknown = true;
                continue;
            }
            if bytes.split(|b| *b == 0).any(|field| {
                field
                    .strip_prefix(b"SteamAppId=")
                    .or_else(|| field.strip_prefix(b"SteamGameId="))
                    .is_some_and(|value| !value.is_empty() && value != b"0")
            }) {
                return Ok(SteamState::GamesRunning);
            }
        }
        if !ctx
            .processes
            .running(&[
                root.join("ubuntu12_32/steam"),
                root.join("ubuntu12_64/steamwebhelper"),
            ])?
            .is_empty()
        {
            return Ok(SteamState::ClientRunning);
        }
        Ok(if unknown {
            SteamState::Unknown
        } else {
            SteamState::Closed
        })
    }
    fn start(&self, root: &Path, unit: &str, ctx: &ExecutionContext) -> AppResult<()> {
        let canonical = fs::canonicalize(root)?;
        let normal = ctx.paths.home.join(".steam/root");
        if normal.exists() && fs::canonicalize(normal)? != canonical {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "El lanzador Steam utiliza otra instalación; no se cambia su ubicación",
            ));
        }
        let executable = root.join("ubuntu12_32/steam");
        let metadata = fs::symlink_metadata(&executable)?;
        if !metadata.is_file()
            || metadata.len() > 64 * 1024 * 1024
            || metadata.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "Cliente Steam no válido",
            ));
        }
        let data = fs::read(executable)?;
        for flag in [b"-forcesteamupdate".as_slice(), b"-silent".as_slice()] {
            if !data.split(|b| *b == 0).any(|item| item == flag) {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    "Los flags de actualización no están comprobados en este cliente; abre Steam desde su lanzador para actualizar",
                ));
            }
        }
        crate::helper::validate::trusted_root_path(Path::new("/usr/bin/steam"))?;
        let spec = CommandSpec::new("/usr/bin/systemd-run").args([
            "--user".into(),
            "--collect".into(),
            format!("--unit={unit}"),
            "--property=KillMode=process".into(),
            "--".into(),
            "/usr/bin/steam".into(),
            "-forcesteamupdate".into(),
            "-silent".into(),
        ]);
        let result = ctx.provider.commands.run(&spec)?;
        if result.code != 0 {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "No se pudo iniciar el actualizador oficial de Steam",
            ));
        }
        Ok(())
    }
    fn wait(
        &self,
        root: &Path,
        channel: &str,
        expected: &str,
        before: &str,
        _: &ExecutionContext,
    ) -> AppResult<()> {
        let until = Instant::now() + Duration::from_secs(180);
        while Instant::now() < until {
            if verify_client_installation(root, channel, expected, Some(before)).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        Err(AppError::new(
            ErrorCode::TransactionFailed,
            "Steam no confirmó el build en el plazo de observación; no se cierra su unidad ni se anuncia Updated",
        ))
    }
}
pub fn run_steam(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<TargetResult> {
    if step.kind != StepKind::SteamUpdate || step.targets.len() != 1 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Paso Steam no válido",
        ));
    }
    let report = ctx.report()?;
    let target = report
        .apps
        .iter()
        .flat_map(|a| &a.components)
        .find(|t| step.targets.contains(&t.id))
        .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "El cliente Steam cambió"))?;
    let Locator::SteamClient { root, channel } = &target.locator else {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "El destino ya no es el cliente Steam",
        ));
    };
    let available = report
        .checks
        .iter()
        .find(|c| c.target_id == target.id && !c.stale)
        .and_then(|c| c.available.as_ref())
        .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "Falta el build Steam confirmado"))?;
    if available.kind != VersionKind::Product
        || available.raw.is_empty()
        || !available.raw.bytes().all(|c| c.is_ascii_digit())
        || step.downloads.len() != 1
        || step.downloads[0].url != manifest_url(channel)?
    {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "El plan no coincide con el canal/build Steam",
        ));
    }
    if detect_channel(root)? != *channel {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Cambió el canal Steam; prepara otro plan",
        ));
    }
    let old = manifest(root, channel)?;
    match crate::alpm::arch_compare(&available.raw, &old.raw)? {
        std::cmp::Ordering::Equal => {
            return Ok(TargetResult {
                target_id: target.id.clone(),
                outcome: Outcome::Current,
                old_version: Some(old.clone()),
                new_version: Some(old),
                error: None,
            });
        }
        std::cmp::Ordering::Less => {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "No se permite degradar el cliente Steam",
            ));
        }
        _ => (),
    }
    if ctx.steam.state(root, ctx)? != SteamState::Closed {
        return Err(AppError::new(
            ErrorCode::Busy,
            "Cierra Steam y sus juegos manualmente; el estado activo o incierto requiere decisión",
        ));
    }
    let before = installation_hash(root)?;
    if ctx.operation_id.0.len() > 64
        || !ctx
            .operation_id
            .0
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "ID de unidad Steam no válido",
        ));
    }
    let unit = format!("app-updates-steam-{}", ctx.operation_id.0);
    ctx.steam.start(root, &unit, ctx)?;
    ctx.steam
        .wait(root, channel, &available.raw, &before, ctx)?;
    let installed = verify_client_installation(root, channel, &available.raw, Some(&before))?;
    Ok(TargetResult {
        target_id: target.id.clone(),
        outcome: Outcome::Updated,
        old_version: Some(old),
        new_version: Some(installed),
        error: None,
    })
}
