use crate::{domain::*, engine::EngineMetadata, execution::EventSink, paths::Paths};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallReceipt {
    pub version: String,
    pub source_digest: String,
    pub artifact_digest: String,
    pub owned_files: Vec<PathBuf>,
    pub shell_backup: Option<PathBuf>,
    pub prepared_at: u64,
}
#[derive(Debug, Clone)]
pub struct EngineArtifacts {
    pub engine: PathBuf,
    pub helper: PathBuf,
    pub metadata: EngineMetadata,
}
pub trait InstallBackend {
    fn build(
        &self,
        source: &Path,
        paths: &Paths,
        sink: &dyn EventSink,
    ) -> AppResult<EngineArtifacts>;
    fn install_helper(
        &self,
        artifacts: &EngineArtifacts,
        source: &Path,
        paths: &Paths,
    ) -> AppResult<()>;
    fn shell(&self, args: &[&str]) -> AppResult<()>;
    fn remove_helper(&self, paths: &Paths) -> AppResult<()>;
}
pub fn engine_compatible(
    plugin_version: &str,
    helper_version: &str,
    protocol: u32,
    abi_major: u32,
) -> bool {
    plugin_version == env!("CARGO_PKG_VERSION")
        && helper_version == plugin_version
        && protocol == 1
        && env!("APP_UPDATES_ALPM_VERSION")
            .split('.')
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            == Some(abi_major)
}
fn idle(paths: &Paths) -> AppResult<()> {
    if crate::store::Store::new(paths.clone())
        .load()?
        .operations
        .values()
        .any(|o| !crate::jobs::terminal(&o.phase))
    {
        return Err(AppError::new(
            ErrorCode::Busy,
            "Hay un trabajo activo; espera su resultado antes de preparar o retirar el plugin",
        ));
    }
    Ok(())
}
const ALLOWED: &[&str] = &[
    "manifest.json",
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "README.md",
    "LICENSE",
    "src",
    "ui",
    "catalog",
    "native",
    "packaging",
    "scripts",
    "bin",
];
fn validate_tree(path: &Path) -> AppResult<()> {
    let fd = rustix::fs::openat2(
        rustix::fs::CWD,
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| {
        AppError::new(
            ErrorCode::UnsafeArtifact,
            "La fuente del plugin contiene enlaces o una ruta no segura",
        )
    })?;
    let meta = fs::File::from(fd).metadata()?;
    if meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o022 != 0
        || (!meta.is_file() && !meta.is_dir())
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Las fuentes deben pertenecer al usuario y no ser modificables por otros",
        ));
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            validate_tree(&entry?.path())?;
        }
    } else if meta.nlink() != 1 || meta.len() > 128 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Archivo de fuente no válido",
        ));
    }
    Ok(())
}
pub fn validate_source(source: &Path) -> AppResult<()> {
    if !source.is_absolute() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "La fuente necesita una ruta absoluta",
        ));
    }
    for required in ["manifest.json", "Cargo.toml", "Cargo.lock"] {
        if !source.join(required).is_file() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                format!("Falta {required}; no se prepara un motor con dependencias sin fijar"),
            ));
        }
    }
    for name in ALLOWED {
        let path = source.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            validate_tree(&path)?;
        }
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
    if manifest["id"] != "korvo.app-updates" || manifest["version"] != env!("CARGO_PKG_VERSION") {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Manifiesto de otro plugin o versión",
        ));
    }
    Ok(())
}
fn safe_directory(path: &Path) -> AppResult<()> {
    use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let mut current = open("/", flags, Mode::empty()).map_err(std::io::Error::from)?;
    for component in path.components() {
        let name = match component {
            std::path::Component::RootDir => continue,
            std::path::Component::Normal(n) => n,
            _ => {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "Directorio de instalación inválido",
                ));
            }
        };
        current = match openat(&current, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => {
                mkdirat(&current, name, Mode::RUSR | Mode::WUSR | Mode::XUSR)
                    .map_err(std::io::Error::from)?;
                openat(&current, name, flags, Mode::empty()).map_err(std::io::Error::from)?
            }
            Err(_) => {
                return Err(AppError::new(
                    ErrorCode::UnsafeArtifact,
                    "El destino de instalación contiene enlaces",
                ));
            }
        };
    }
    let meta = fs::File::from(current).metadata()?;
    if meta.uid() != rustix::process::geteuid().as_raw() || meta.mode() & 0o022 != 0 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "El destino de instalación tiene permisos no seguros",
        ));
    }
    Ok(())
}
fn copy_tree(source: &Path, target: &Path) -> AppResult<()> {
    validate_tree(source)?;
    let meta = fs::symlink_metadata(source)?;
    if meta.is_dir() {
        fs::create_dir(target)?;
        fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, target)?;
        fs::set_permissions(
            target,
            fs::Permissions::from_mode(if meta.mode() & 0o111 != 0 {
                0o700
            } else {
                0o600
            }),
        )?;
    }
    Ok(())
}
pub fn install_with_backend(
    source: &Path,
    paths: &Paths,
    backend: &dyn InstallBackend,
    sink: &dyn EventSink,
) -> AppResult<InstallReceipt> {
    idle(paths)?;
    let _lease = crate::jobs::worker::worker_lock(paths)?;
    validate_source(source)?;
    let artifacts = backend.build(source, paths, sink)?;
    crate::engine::compatible(&artifacts.metadata)?;
    let parent = paths
        .plugin
        .parent()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Plugin sin directorio padre"))?;
    safe_directory(parent)?;
    let stage = parent.join(format!(".app-updates-stage-{}", uuid::Uuid::new_v4()));
    crate::store::private_dir(&stage)?;
    let preparation = (|| -> AppResult<InstallReceipt> {
        for name in ALLOWED {
            if *name == "bin" {
                continue;
            }
            let path = source.join(name);
            if path.exists() {
                copy_tree(&path, &stage.join(name))?;
            }
        }
        crate::store::private_dir(&stage.join("bin"))?;
        copy_tree(&artifacts.engine, &stage.join("bin/app-updates"))?;
        fs::set_permissions(
            stage.join("bin/app-updates"),
            fs::Permissions::from_mode(0o700),
        )?;
        let helper_hash = crate::providers::telegram::binary_hash(&artifacts.helper)?;
        let engine_hash = crate::providers::telegram::binary_hash(&stage.join("bin/app-updates"))?;
        backend.install_helper(&artifacts, source, paths)?;
        idle(paths)?;
        let shell = paths.home.join(".config/omarchy/shell.json");
        let shell_backup = if shell.exists() {
            crate::store::check_plain(&shell)?;
            let backup = paths
                .state
                .join("installation-backups")
                .join(format!("shell-{}.json", uuid::Uuid::new_v4()));
            crate::store::private_dir(backup.parent().unwrap())?;
            fs::copy(&shell, &backup)?;
            fs::set_permissions(&backup, fs::Permissions::from_mode(0o600))?;
            Some(backup)
        } else {
            None
        };
        let previous = parent.join(format!(".app-updates-previous-{}", uuid::Uuid::new_v4()));
        if paths.plugin.exists() {
            validate_tree(&paths.plugin)?;
            fs::rename(&paths.plugin, &previous)?;
        }
        if let Err(error) = fs::rename(&stage, &paths.plugin) {
            if previous.exists() {
                let _ = fs::rename(&previous, &paths.plugin);
            }
            return Err(error.into());
        }
        let receipt = InstallReceipt {
            version: env!("CARGO_PKG_VERSION").into(),
            source_digest: artifacts.metadata.source_digest,
            artifact_digest: format!("{engine_hash}:{helper_hash}"),
            owned_files: vec![
                paths.plugin.clone(),
                paths.helper.clone(),
                "/usr/share/polkit-1/actions/org.korvo.app-updates.policy".into(),
            ],
            shell_backup,
            prepared_at: crate::now(),
        };
        let activated = backend
            .shell(&["rescan"])
            .and_then(|()| backend.shell(&["enable", "korvo.app-updates"]));
        if let Err(error) = activated {
            fs::rename(&paths.plugin, &stage)?;
            if previous.exists() {
                fs::rename(&previous, &paths.plugin)?;
            }
            if let Some(backup) = &receipt.shell_backup {
                let restored =
                    shell.with_file_name(format!(".shell-restore-{}", uuid::Uuid::new_v4()));
                fs::copy(backup, &restored)?;
                fs::set_permissions(&restored, fs::Permissions::from_mode(0o600))?;
                fs::rename(restored, &shell)?;
            }
            let _ = backend.shell(&["rescan"]);
            return Err(error);
        }
        crate::store::atomic_json(&paths.state.join("installation.json"), &receipt)?;
        Ok(receipt)
    })();
    if stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    preparation
}
pub fn remove_with_backend(
    paths: &Paths,
    backend: &dyn InstallBackend,
) -> AppResult<InstallReceipt> {
    idle(paths)?;
    let _lease = crate::jobs::worker::worker_lock(paths)?;
    let bytes = crate::store::read_private(&paths.state.join("installation.json"), 1024 * 1024)?
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Falta el recibo de instalación"))?;
    let receipt: InstallReceipt = serde_json::from_slice(&bytes)?;
    backend.remove_helper(paths)?;
    backend.shell(&["disable", "korvo.app-updates"])?;
    if paths.plugin.exists() {
        validate_tree(&paths.plugin)?;
        fs::remove_dir_all(&paths.plugin)?;
    }
    backend.shell(&["rescan"])?;
    Ok(receipt)
}
#[derive(Default)]
pub struct NativeInstall {
    pub prebuilt: Option<EngineArtifacts>,
}
impl InstallBackend for NativeInstall {
    fn build(
        &self,
        source: &Path,
        paths: &Paths,
        sink: &dyn EventSink,
    ) -> AppResult<EngineArtifacts> {
        if let Some(artifacts) = &self.prebuilt {
            return Ok(artifacts.clone());
        }
        let cargo = paths
            .home
            .join(".rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo");
        validate_tree(&cargo)?;
        let target = paths.state.join("build/native");
        crate::store::private_dir(&target)?;
        sink.send(OperationEvent {
            operation_id: "engine-preparation".into(),
            seq: 1,
            phase: Phase::Building,
            message: "Compilando fuentes fijadas con Cargo.lock; dos procesos de compilación"
                .into(),
            progress: None,
            detail: None,
        })?;
        let mut spec = crate::io::command::CommandSpec::new(cargo).args([
            "build".into(),
            "--release".into(),
            "--locked".into(),
            "--bins".into(),
            "--manifest-path".into(),
            source.join("Cargo.toml").to_string_lossy().into(),
        ]);
        spec.cwd = Some(source.into());
        spec.timeout = std::time::Duration::from_secs(3600);
        spec.env.insert(
            "PATH".into(),
            format!("{}/.cargo/bin:/usr/bin", paths.home.display()),
        );
        spec.env.insert("CARGO_BUILD_JOBS".into(), "2".into());
        spec.env
            .insert("CARGO_TARGET_DIR".into(), target.to_string_lossy().into());
        let result = crate::io::command::CommandRunner::default().run(&spec)?;
        private_log(
            &paths.state.join("preparation-cargo.log"),
            &format!("{}\n{}", result.stdout, result.stderr),
        )?;
        if result.code != 0 {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "La compilación falló; consulta preparation-cargo.log. Se conserva la instalación anterior",
            ));
        }
        built_artifacts(&target.join("release"))
    }
    fn install_helper(
        &self,
        artifacts: &EngineArtifacts,
        source: &Path,
        paths: &Paths,
    ) -> AppResult<()> {
        let package = build_helper_package(artifacts, source, paths)?;
        graphical_pacman(
            &[
                "-U",
                "--noconfirm",
                "--",
                package.to_str().ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidInput, "Ruta del paquete no válida")
                })?,
            ],
            paths,
        )?;
        crate::engine::verify_helper(paths)?;
        crate::helper::validate::trusted_root_path(Path::new(
            "/usr/share/polkit-1/actions/org.korvo.app-updates.policy",
        ))
    }
    fn shell(&self, args: &[&str]) -> AppResult<()> {
        let (program, args) = match args {
            ["rescan"] => (
                "/usr/share/omarchy/bin/omarchy-shell",
                vec!["shell", "rescanPlugins"],
            ),
            ["enable", "korvo.app-updates"] | ["disable", "korvo.app-updates"] => (
                "/usr/share/omarchy/bin/omarchy",
                vec!["plugin", args[0], "korvo.app-updates"],
            ),
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Acción del shell no permitida",
                ));
            }
        };
        let result = crate::io::command::CommandRunner::default().run(
            &crate::io::command::CommandSpec::new(program).args(args.into_iter().map(String::from)),
        )?;
        if result.code != 0 {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "El shell no pudo activar el cambio; se conserva el respaldo",
            ));
        }
        Ok(())
    }
    fn remove_helper(&self, paths: &Paths) -> AppResult<()> {
        graphical_pacman(
            &["-R", "--noconfirm", "--", "omarchy-app-updates-helper"],
            paths,
        )
    }
}
fn private_log(path: &Path, text: &str) -> AppResult<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    crate::store::private_dir(path.parent().unwrap())?;
    crate::store::check_plain(path)?;
    let temporary = path.with_file_name(format!(".log-{}", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}
pub fn built_artifacts(release: &Path) -> AppResult<EngineArtifacts> {
    let engine = release.join("app-updates");
    let helper = release.join("app-updates-helper");
    validate_tree(&engine)?;
    validate_tree(&helper)?;
    let metadata = crate::engine::inspect(&engine)?;
    crate::engine::compatible(&metadata)?;
    crate::engine::compatible(&crate::engine::inspect(&helper)?)?;
    Ok(EngineArtifacts {
        engine,
        helper,
        metadata,
    })
}
fn graphical_pacman(args: &[&str], paths: &Paths) -> AppResult<()> {
    crate::helper::validate::trusted_root_path(Path::new("/usr/bin/pkexec"))?;
    crate::helper::validate::trusted_root_path(Path::new("/usr/bin/pacman"))?;
    // No timeout or kill after confirmation: pacman owns its complete transaction and hooks.
    let output = std::process::Command::new("/usr/bin/pkexec")
        .args(["--disable-internal-agent", "/usr/bin/pacman"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()?;
    private_log(
        &paths.state.join("preparation-pacman.log"),
        &format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(127);
        return Err(if matches!(code, 126 | 127) {
            crate::helper::client::authorization_error(code)
        } else {
            AppError::new(
                ErrorCode::TransactionFailed,
                "No se completó la instalación del paquete propio; conflictos, firmas o dependencias requieren revisión. Consulta preparation-pacman.log",
            )
        });
    }
    Ok(())
}
pub fn validate_package_contents(package: &crate::alpm::ArchiveInfo) -> AppResult<()> {
    let payload = [
        "usr/lib/omarchy-app-updates/app-updates-helper",
        "usr/share/polkit-1/actions/org.korvo.app-updates.policy",
        "usr/share/licenses/omarchy-app-updates-helper/LICENSE",
    ];
    if package.name != "omarchy-app-updates-helper"
        || package.version != format!("{}-1", env!("CARGO_PKG_VERSION"))
        || package.architecture != "x86_64"
        || payload[..2]
            .iter()
            .any(|p| !package.files.iter().any(|f| f == p))
        || package.files.iter().any(|f| {
            if f.ends_with('/') {
                !payload.iter().any(|p| p.starts_with(f))
            } else {
                !payload.contains(&f.as_str())
            }
        })
        || package
            .depends
            .iter()
            .any(|d| !["pacman", "flatpak", "libarchive"].contains(&d.as_str()))
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "El paquete del plugin contiene archivos, identidad o dependencias fuera de su alcance propio",
        ));
    }
    Ok(())
}
pub fn build_helper_package(
    artifacts: &EngineArtifacts,
    source: &Path,
    paths: &Paths,
) -> AppResult<PathBuf> {
    validate_tree(&artifacts.helper)?;
    let dir = paths
        .state
        .join("helper-packages")
        .join(uuid::Uuid::new_v4().to_string());
    crate::store::private_dir(&dir)?;
    let helper = dir.join("app-updates-helper");
    copy_tree(&artifacts.helper, &helper)?;
    for file in ["org.korvo.app-updates.policy", "LICENSE"] {
        let from = if file == "LICENSE" {
            source.join(file)
        } else {
            source.join("packaging").join(file)
        };
        copy_tree(&from, &dir.join(file))?;
    }
    let mut recipe = fs::read_to_string(source.join("packaging/PKGBUILD"))?;
    for (marker, name) in [
        ("@HELPER_SHA256@", "app-updates-helper"),
        ("@POLICY_SHA256@", "org.korvo.app-updates.policy"),
        ("@LICENSE_SHA256@", "LICENSE"),
    ] {
        recipe = recipe.replace(
            marker,
            &crate::providers::telegram::binary_hash(&dir.join(name))?,
        );
    }
    fs::write(dir.join("PKGBUILD"), recipe)?;
    let mut spec =
        crate::io::command::CommandSpec::new("/usr/bin/makepkg").args(["--noconfirm".into()]);
    spec.cwd = Some(dir.clone());
    for name in ["PKGDEST", "SRCDEST", "BUILDDIR", "LOGDEST", "SRCPKGDEST"] {
        spec.env.insert(name.into(), dir.to_string_lossy().into());
    }
    spec.timeout = std::time::Duration::from_secs(180);
    let output = crate::io::command::CommandRunner::default().run(&spec)?;
    private_log(
        &paths.state.join("preparation-package.log"),
        &format!("{}\n{}", output.stdout, output.stderr),
    )?;
    if output.code != 0 {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            "No se pudo empaquetar el ejecutor; consulta preparation-package.log",
        ));
    }
    let package = dir.join(format!(
        "omarchy-app-updates-helper-{}-1-x86_64.pkg.tar.zst",
        env!("CARGO_PKG_VERSION")
    ));
    let runner = crate::io::command::CommandRunner::default();
    let cfg = crate::alpm::AlpmConfig::load(&paths.pacman_conf, &runner)?;
    let reader = crate::alpm::AlpmReader::open(&cfg, &cfg.dbpath)?;
    for dep in ["pacman", "flatpak", "libarchive"] {
        if !reader.satisfies(dep)? {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Faltan dependencias instaladas del ejecutor; no se sincronizan paquetes parcialmente",
            ));
        }
    }
    let sig = crate::alpm::transaction::signature_level(
        &cfg.local_siglevel,
        crate::alpm::bindings::ALPM_SIG_PACKAGE | crate::alpm::bindings::ALPM_SIG_PACKAGE_OPTIONAL,
    )?;
    validate_package_contents(&reader.archive(&package, sig)?)?;
    let listing = runner.run(
        &crate::io::command::CommandSpec::new("/usr/bin/bsdtar")
            .args(["-tf".into(), package.to_string_lossy().into()]),
    )?;
    if listing.code != 0
        || listing.stdout.lines().any(|name| {
            name == ".INSTALL"
                || name == ".CHANGELOG"
                || name.starts_with('/')
                || name.split('/').any(|c| c == "..")
        })
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "El paquete contiene un script de instalación o rutas inesperadas",
        ));
    }
    let types = runner.run(
        &crate::io::command::CommandSpec::new("/usr/bin/bsdtar")
            .args(["-tvf".into(), package.to_string_lossy().into()]),
    )?;
    if types.code != 0
        || types
            .stdout
            .lines()
            .any(|line| !line.starts_with('-') && !line.starts_with('d'))
    {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "El paquete contiene enlaces o nodos especiales",
        ));
    }
    crate::store::atomic_json(
        &dir.join("verified-package.json"),
        &serde_json::json!({"package":package,"sha256":crate::providers::telegram::binary_hash(&package)?,"files":reader.archive(&package,sig)?.files}),
    )?;
    Ok(package)
}
pub fn prepare_engine(
    plugin: &Path,
    paths: &Paths,
    sink: &dyn EventSink,
) -> AppResult<InstallReceipt> {
    install_with_backend(plugin, paths, &NativeInstall::default(), sink)
}
pub fn install_plugin(source: &Path, paths: &Paths) -> AppResult<InstallReceipt> {
    prepare_engine(source, paths, &crate::helper::SilentSink)
}
pub fn remove_plugin(paths: &Paths) -> AppResult<InstallReceipt> {
    remove_with_backend(paths, &NativeInstall::default())
}
pub fn finish_engine(source: &Path, paths: &Paths) -> AppResult<InstallReceipt> {
    let release = std::env::current_exe()?
        .parent()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Binario sin directorio"))?
        .to_path_buf();
    let artifacts = built_artifacts(&release)?;
    install_with_backend(
        source,
        paths,
        &NativeInstall {
            prebuilt: Some(artifacts),
        },
        &crate::helper::SilentSink,
    )
}
