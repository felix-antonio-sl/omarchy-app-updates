use crate::{
    domain::*,
    execution::ExecutionContext,
    providers::telegram::{TelegramReceipt, binary_hash, installed_version},
};
use std::{
    collections::BTreeSet,
    ffi::CStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    ptr::NonNull,
};
#[allow(
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    dead_code,
    clippy::all
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/archive.rs"));
}
pub trait ActivationBackend: Send + Sync {
    fn rename(&self, from: &Path, to: &Path) -> AppResult<()>;
}
pub struct FileActivation;
impl ActivationBackend for FileActivation {
    fn rename(&self, from: &Path, to: &Path) -> AppResult<()> {
        fs::rename(from, to)?;
        Ok(())
    }
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StagedTelegram {
    pub root: PathBuf,
    pub payload: PathBuf,
    pub previous: PathBuf,
    pub version: String,
    pub old_version: Option<Version>,
    pub old_hash: String,
    pub old_updater_hash: String,
    pub binary_hash: String,
    pub updater_hash: String,
    pub archive_hash: String,
    pub source: String,
    pub target_id: TargetId,
    pub operation_id: OperationId,
    pub root_device: u64,
    pub root_inode: u64,
    pub vendor_signature_verified: bool,
    pub current: bool,
}
fn unsafe_artifact(message: &str) -> AppError {
    AppError::new(ErrorCode::UnsafeArtifact, message)
}
fn user_file(path: &Path) -> AppResult<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file()
        || m.file_type().is_symlink()
        || m.uid() != rustix::process::geteuid().as_raw()
        || m.nlink() != 1
        || m.mode() & 0o022 != 0
    {
        return Err(unsafe_artifact(
            "La instalación local contiene un ejecutable enlazado o con permisos inseguros",
        ));
    }
    Ok(())
}
fn root_for_step(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<PathBuf> {
    if step.kind != StepKind::TelegramReplace || step.targets.len() != 1 {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "El paso no corresponde a una instalación Telegram",
        ));
    }
    let report = ctx.report()?;
    let root = report
        .apps
        .iter()
        .flat_map(|a| &a.components)
        .find(|t| step.targets.contains(&t.id))
        .and_then(|t| match &t.locator {
            Locator::TelegramLocal { root } => Some(root.clone()),
            _ => None,
        })
        .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "La instalación Telegram cambió"))?;
    if !root.starts_with(&ctx.paths.home)
        || root == ctx.paths.home
        || root.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(unsafe_artifact(
            "Telegram debe pertenecer a la instalación local del operador",
        ));
    }
    let fd = rustix::fs::openat2(
        rustix::fs::CWD,
        &root,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| unsafe_artifact("Directorio Telegram enlazado"))?;
    let m = File::from(fd).metadata()?;
    if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o022 != 0 {
        return Err(unsafe_artifact(
            "El directorio Telegram no tiene su propietario esperado",
        ));
    }
    for name in ["Telegram", "Updater"] {
        user_file(&root.join(name))?;
    }
    Ok(root)
}
fn version_from_step(step: &PlanStep) -> AppResult<String> {
    if step.downloads.len() != 1 {
        return Err(unsafe_artifact("Telegram requiere un origen fijo"));
    }
    let source = &step.downloads[0].url;
    let version = source
        .strip_prefix("https://td.telegram.org/linux-x64/td-setup-linux-x64-")
        .and_then(|s| s.strip_suffix(".tar.xz"))
        .ok_or_else(|| {
            unsafe_artifact("El origen Telegram no coincide con un release fijo oficial")
        })?;
    if version.is_empty()
        || version.len() > 40
        || version
            .split('.')
            .any(|s| s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err(unsafe_artifact("Versión Telegram no válida"));
    }
    Ok(version.into())
}
struct Archive(NonNull<ffi::archive>);
impl Drop for Archive {
    fn drop(&mut self) {
        unsafe { ffi::archive_read_free(self.0.as_ptr()) };
    }
}
/// Read only the two regular executables from Telegram's tar.xz; never extract directory metadata or links.
pub fn extract_telegram(archive: &Path, destination: &Path) -> AppResult<()> {
    let mut input = File::open(archive)?;
    let mut magic = [0u8; 6];
    input.read_exact(&mut magic)?;
    if magic != [0xfd, b'7', b'z', b'X', b'Z', 0] {
        return Err(unsafe_artifact("La descarga no es un archive XZ"));
    }
    use std::io::{Seek, SeekFrom};
    input.seek(SeekFrom::Start(0))?;
    crate::execution::aur::private_directory(destination)?;
    let reader = Archive(
        NonNull::new(unsafe { ffi::archive_read_new() })
            .ok_or_else(|| unsafe_artifact("No se pudo preparar el lector de archive"))?,
    );
    let a = reader.0.as_ptr();
    unsafe {
        if ffi::archive_read_support_filter_xz(a) != 0
            || ffi::archive_read_support_format_tar(a) != 0
            || ffi::archive_read_open_fd(a, input.as_raw_fd(), 65536) != 0
        {
            return Err(unsafe_artifact("No se pudo abrir el archive Telegram"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut count = 0;
    let mut total = 0u64;
    loop {
        let mut entry = std::ptr::null_mut();
        let result = unsafe { ffi::archive_read_next_header(a, &mut entry) };
        if result == ffi::ARCHIVE_EOF as i32 {
            break;
        }
        if result != 0 || entry.is_null() {
            return Err(unsafe_artifact("Archive Telegram incompleto o dañado"));
        }
        count += 1;
        if count > 32 {
            return Err(unsafe_artifact(
                "Demasiadas entradas en el archive Telegram",
            ));
        }
        let raw = unsafe { ffi::archive_entry_pathname(entry) };
        if raw.is_null() {
            return Err(unsafe_artifact("Entrada Telegram sin nombre"));
        }
        let name = unsafe { CStr::from_ptr(raw) }
            .to_str()
            .map_err(|_| unsafe_artifact("Ruta de archive no válida"))?;
        let path = Path::new(name);
        if path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || unsafe {
                !ffi::archive_entry_symlink(entry).is_null()
                    || !ffi::archive_entry_hardlink(entry).is_null()
            }
        {
            return Err(unsafe_artifact("El archive contiene traversal o enlaces"));
        }
        let kind = unsafe { ffi::archive_entry_filetype(entry) };
        if kind == ffi::APP_AE_IFDIR && (name == "Telegram/" || name == "Telegram") {
            continue;
        }
        if kind != ffi::APP_AE_IFREG
            || !matches!(name, "Telegram/Telegram" | "Telegram/Updater")
            || !seen.insert(name.to_string())
        {
            return Err(unsafe_artifact(
                "El archive no contiene únicamente los ejecutables esperados",
            ));
        }
        let size = unsafe { ffi::archive_entry_size(entry) };
        if size <= 0 || size > 1024 * 1024 * 1024 {
            return Err(unsafe_artifact(
                "Tamaño de ejecutable Telegram no permitido",
            ));
        }
        let filename = path.file_name().unwrap();
        let dest = destination.join(filename);
        let mut out = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o700)
            .open(dest)?;
        let mut bytes = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = unsafe { ffi::archive_read_data(a, buffer.as_mut_ptr().cast(), buffer.len()) };
            if n == 0 {
                break;
            }
            if n < 0 {
                return Err(unsafe_artifact("No se pudo leer el contenido Telegram"));
            }
            bytes += n as u64;
            total += n as u64;
            if bytes > size as u64 || total > 1536 * 1024 * 1024 {
                return Err(unsafe_artifact(
                    "El contenido descomprimido excede los límites",
                ));
            }
            out.write_all(&buffer[..n as usize])?;
        }
        if bytes != size as u64 {
            return Err(unsafe_artifact("Ejecutable Telegram truncado"));
        }
        out.sync_all()?;
    }
    if seen.len() != 2 {
        return Err(unsafe_artifact("Falta Telegram o Updater en la descarga"));
    }
    Ok(())
}
fn verify_elf(path: &Path, ctx: &ExecutionContext) -> AppResult<()> {
    let mut file = File::open(path)?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header)?;
    if &header[..4] != b"\x7fELF"
        || header[4] != 2
        || header[5] != 1
        || u16::from_le_bytes([header[18], header[19]]) != 62
        || !matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
    {
        return Err(unsafe_artifact(
            "Telegram no contiene un ejecutable ELF compatible con x86_64",
        ));
    }
    let output = ctx.provider.commands.run(
        &crate::io::command::CommandSpec::new("/usr/bin/readelf")
            .args(["-d".into(), path.to_string_lossy().into()]),
    )?;
    if output.code != 0 {
        return Err(unsafe_artifact(
            "No se pudo comprobar el ejecutable Telegram",
        ));
    }
    let libraries = ctx
        .provider
        .commands
        .run(&crate::io::command::CommandSpec::new("/usr/bin/ldconfig").args(["-p".into()]))?;
    if libraries.code != 0 {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "No se pudo comprobar la disponibilidad de bibliotecas",
        ));
    }
    for line in output
        .stdout
        .lines()
        .filter(|line| line.contains("(NEEDED)"))
    {
        let library = line
            .split_once('[')
            .and_then(|(_, s)| s.split_once(']').map(|(name, _)| name))
            .ok_or_else(|| unsafe_artifact("Dependencia ELF no válida"))?;
        if !libraries
            .stdout
            .lines()
            .any(|row| row.split_whitespace().next() == Some(library))
        {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                format!("Telegram requiere la biblioteca {library} antes de activarse"),
            ));
        }
    }
    Ok(())
}
pub fn stage_telegram(step: &PlanStep, ctx: &ExecutionContext) -> AppResult<StagedTelegram> {
    crate::execution::aur::ensure_unprivileged(rustix::process::geteuid().as_raw())?;
    let root = root_for_step(step, ctx)?;
    let version = version_from_step(step)?;
    let old = installed_version(&root)?.ok_or_else(|| {
        AppError::new(
            ErrorCode::PlanChanged,
            "Se requiere confirmar la versión asociada al binario Telegram instalado",
        )
    })?;
    if crate::alpm::arch_compare(&version, &old.raw)? == std::cmp::Ordering::Less {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "No se permite degradar Telegram",
        ));
    }
    let current = old.raw == version;
    let source = step.downloads[0].url.clone();
    let work = ctx
        .paths
        .state
        .join("telegram")
        .join(uuid::Uuid::new_v4().to_string());
    crate::execution::aur::private_directory(&work)?;
    let payload = work.join("payload");
    let previous = work.join("previous");
    crate::execution::aur::private_directory(&previous)?;
    let root_metadata = fs::metadata(&root)?;
    let mut stage = StagedTelegram {
        root_device: root_metadata.dev(),
        root_inode: root_metadata.ino(),
        old_hash: binary_hash(&root.join("Telegram"))?,
        old_updater_hash: binary_hash(&root.join("Updater"))?,
        root,
        payload,
        previous,
        version,
        old_version: Some(old),
        binary_hash: String::new(),
        updater_hash: String::new(),
        archive_hash: String::new(),
        source,
        target_id: step.targets[0].clone(),
        operation_id: ctx.operation_id.clone(),
        vendor_signature_verified: false,
        current,
    };
    if current {
        return Ok(stage);
    }
    let response = ctx.provider.http.get(
        &stage.source,
        &crate::io::http::DownloadPolicy::for_hosts(&["td.telegram.org"], 512 * 1024 * 1024),
    )?;
    let final_url = reqwest::Url::parse(&response.url)
        .map_err(|_| unsafe_artifact("El release Telegram redirigió a una URL no válida"))?;
    let expected_url = reqwest::Url::parse(&stage.source)
        .map_err(|_| unsafe_artifact("Origen Telegram no válido"))?;
    if final_url.path() != expected_url.path() {
        return Err(unsafe_artifact(
            "Telegram redirigió a una versión distinta; requiere preparar otro plan",
        ));
    }
    use sha2::{Digest, Sha256};
    stage.archive_hash = format!("{:x}", Sha256::digest(&response.bytes));
    if step.downloads[0]
        .expected_sha256
        .as_ref()
        .is_some_and(|hash| hash != &stage.archive_hash)
    {
        return Err(unsafe_artifact(
            "La descarga cambió respecto del checksum esperado",
        ));
    }
    let archive = work.join("telegram.tar.xz");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&archive)?;
    file.write_all(&response.bytes)?;
    file.sync_all()?;
    extract_telegram(&archive, &stage.payload)?;
    for name in ["Telegram", "Updater"] {
        verify_elf(&stage.payload.join(name), ctx)?;
    }
    stage.binary_hash = binary_hash(&stage.payload.join("Telegram"))?;
    stage.updater_hash = binary_hash(&stage.payload.join("Updater"))?;
    Ok(stage)
}
fn receipt(path: &Path, value: &TelegramReceipt) -> AppResult<()> {
    let parent = path.parent().unwrap();
    let temp = parent.join(format!(".apps-receipt-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(temp, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn activate_telegram(
    stage: &StagedTelegram,
    ctx: &ExecutionContext,
) -> AppResult<TargetResult> {
    if stage.operation_id != ctx.operation_id {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "El staging pertenece a otra operación",
        ));
    }
    let fd = rustix::fs::openat2(
        rustix::fs::CWD,
        &stage.root,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| unsafe_artifact("La instalación Telegram se convirtió en un enlace"))?;
    let root_handle = File::from(fd);
    let root_metadata = root_handle.metadata()?;
    if root_metadata.dev() != stage.root_device
        || root_metadata.ino() != stage.root_inode
        || root_metadata.uid() != rustix::process::geteuid().as_raw()
        || root_metadata.mode() & 0o022 != 0
    {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "El directorio Telegram cambió",
        ));
    }
    // Keep directory descriptors alive throughout the swap. Kernel fd paths bind
    // rename/receipt operations to these exact directories even if their names move.
    let open_directory = |path: &Path| -> AppResult<File> {
        let fd = rustix::fs::openat2(
            rustix::fs::CWD,
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| unsafe_artifact("Staging Telegram enlazado"))?;
        let file = File::from(fd);
        let m = file.metadata()?;
        if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o022 != 0 {
            return Err(unsafe_artifact(
                "Staging Telegram con propietario inesperado",
            ));
        }
        Ok(file)
    };
    let previous_handle = open_directory(&stage.previous)?;
    let previous = PathBuf::from(format!("/proc/self/fd/{}", previous_handle.as_raw_fd()));
    let active_root = PathBuf::from(format!("/proc/self/fd/{}", root_handle.as_raw_fd()));
    for name in ["Telegram", "Updater"] {
        user_file(&active_root.join(name))?;
    }

    if binary_hash(&active_root.join("Telegram"))? != stage.old_hash
        || binary_hash(&active_root.join("Updater"))? != stage.old_updater_hash
    {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Telegram cambió desde la preparación",
        ));
    }
    if stage.current {
        return Ok(TargetResult {
            target_id: stage.target_id.clone(),
            outcome: Outcome::Current,
            old_version: stage.old_version.clone(),
            new_version: stage.old_version.clone(),
            error: None,
        });
    }
    let payload_handle = open_directory(&stage.payload)?;
    let payload = PathBuf::from(format!("/proc/self/fd/{}", payload_handle.as_raw_fd()));
    if !ctx
        .processes
        .running(&[active_root.join("Telegram"), active_root.join("Updater")])?
        .is_empty()
    {
        return Err(AppError::new(
            ErrorCode::Busy,
            "Telegram está abierto; requiere decidir cuándo cerrarlo antes de reemplazar ejecutables",
        ));
    }
    for (name, hash) in [
        ("Telegram", &stage.binary_hash),
        ("Updater", &stage.updater_hash),
    ] {
        user_file(&payload.join(name))?;
        if binary_hash(&payload.join(name))? != *hash {
            return Err(unsafe_artifact("Cambió un ejecutable en staging"));
        }
    }
    let old_receipt = fs::read(active_root.join(".app-updates-version.json"))?;
    let mut backed = vec![];
    let mut activated = vec![];
    let result = (|| -> AppResult<()> {
        for name in ["Telegram", "Updater"] {
            ctx.activation
                .rename(&active_root.join(name), &previous.join(name))?;
            backed.push(name);
            ctx.activation
                .rename(&payload.join(name), &active_root.join(name))?;
            activated.push(name);
            fs::set_permissions(active_root.join(name), fs::Permissions::from_mode(0o755))?;
        }
        let named = open_directory(&stage.root)?.metadata()?;
        if named.dev() != stage.root_device || named.ino() != stage.root_inode {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Telegram cambió de directorio durante la activación",
            ));
        }
        if !ctx
            .processes
            .running(&[previous.join("Telegram"), previous.join("Updater")])?
            .is_empty()
        {
            return Err(AppError::new(
                ErrorCode::Busy,
                "Telegram se inició durante el cambio; se restaura la instalación y se requiere otra decisión",
            ));
        }
        receipt(
            &active_root.join(".app-updates-version.json"),
            &TelegramReceipt {
                version: stage.version.clone(),
                binary_sha256: stage.binary_hash.clone(),
            },
        )?;
        if installed_version(&active_root)?.is_none_or(|v| v.raw != stage.version)
            || binary_hash(&active_root.join("Updater"))? != stage.updater_hash
        {
            return Err(unsafe_artifact(
                "La activación no pasó la verificación del binario y metadatos",
            ));
        }
        Ok(())
    })();
    if let Err(error) = result {
        for name in activated.iter().rev() {
            fs::remove_file(active_root.join(name))?;
        }
        for name in backed.iter().rev() {
            ctx.activation
                .rename(&previous.join(name), &active_root.join(name))?;
        }
        let old: TelegramReceipt = serde_json::from_slice(&old_receipt)?;
        receipt(&active_root.join(".app-updates-version.json"), &old)?;
        return Err(error);
    }
    Ok(TargetResult {
        target_id: stage.target_id.clone(),
        outcome: Outcome::Updated,
        old_version: stage.old_version.clone(),
        new_version: Some(Version::product(&stage.version)),
        error: None,
    })
}
