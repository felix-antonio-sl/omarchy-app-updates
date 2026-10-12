use crate::{
    alpm::{AlpmConfig, AlpmReader},
    domain::*,
    execution::ExecutionContext,
    io::command::{CommandRunner, CommandSpec},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::Duration,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeFile {
    pub path: String,
    pub executable: bool,
    pub sha256: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AurDependency {
    pub expression: String,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AurRecipe {
    pub package_base: String,
    pub commit: String,
    pub files: Vec<RecipeFile>,
    pub diff: String,
    pub dependencies: Vec<AurDependency>,
    pub digest: String,
    pub requested: Vec<String>,
    pub packages: BTreeMap<String, String>,
    pub repository: PathBuf,
}
pub use crate::domain::BuiltArtifact;
pub fn ensure_unprivileged(uid: u32) -> AppResult<()> {
    if uid == 0 {
        Err(AppError::new(
            ErrorCode::AuthUnavailable,
            "AUR se compila como usuario; root nunca ejecuta PKGBUILD",
        ))
    } else {
        Ok(())
    }
}
fn invalid(message: &str) -> AppError {
    AppError::new(ErrorCode::UnsafeArtifact, message)
}
pub fn package_name(name: &str) -> bool {
    name.len() <= 120
        && name
            .bytes()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'+'))
}
fn valid_relative(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && !p.is_absolute()
        && p.components()
            .all(|c| matches!(c,Component::Normal(n)if n!=".git"))
}
pub fn private_directory(path: &Path) -> AppResult<()> {
    use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
    if !path.is_absolute() {
        return Err(invalid("El directorio AUR necesita una ruta absoluta"));
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let mut directory = open("/", flags, Mode::empty()).map_err(std::io::Error::from)?;
    for component in path.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => name,
            _ => return Err(invalid("Ruta AUR no válida")),
        };
        let child = match openat(&directory, name, flags, Mode::empty()) {
            Ok(child) => child,
            Err(rustix::io::Errno::NOENT) => {
                match mkdirat(&directory, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                    Ok(()) | Err(rustix::io::Errno::EXIST) => (),
                    Err(e) => return Err(std::io::Error::from(e).into()),
                }
                openat(&directory, name, flags, Mode::empty())
                    .map_err(|_| invalid("Directorio AUR enlazado"))?
            }
            Err(_) => return Err(invalid("Directorio AUR enlazado o inaccesible")),
        };
        directory = child;
    }
    let file = fs::File::from(directory);
    let m = file.metadata()?;
    if m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o077 != 0 {
        return Err(invalid(
            "El directorio AUR debe ser privado y pertenecer al usuario",
        ));
    }
    Ok(())
}
pub(crate) fn git(
    commands: &CommandRunner,
    repository: Option<&Path>,
    args: Vec<String>,
) -> AppResult<String> {
    let mut argv = vec![
        "-c".into(),
        "core.hooksPath=/dev/null".into(),
        "-c".into(),
        "core.fsmonitor=false".into(),
        "-c".into(),
        "core.attributesFile=/dev/null".into(),
        "-c".into(),
        "protocol.file.allow=never".into(),
    ];
    if let Some(dir) = repository {
        argv.extend(["-C".into(), dir.to_string_lossy().into()]);
    }
    argv.extend(args);
    let mut spec = CommandSpec::new("/usr/bin/git").args(argv);
    spec.timeout = Duration::from_secs(120);
    spec.env
        .insert("GIT_CONFIG_GLOBAL".into(), "/dev/null".into());
    spec.env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    spec.env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    let output = commands.run(&spec)?;
    if output.code != 0 {
        return Err(AppError::new(
            ErrorCode::NetworkError,
            "No se pudo obtener/verificar la receta fijada de AUR",
        ));
    }
    Ok(output.stdout)
}
pub fn recipe_digest(recipe: &AurRecipe) -> String {
    let mut files = recipe.files.clone();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut dependencies = recipe.dependencies.clone();
    dependencies.sort_by(|a, b| (&a.kind, &a.expression).cmp(&(&b.kind, &b.expression)));
    let mut requested = recipe.requested.clone();
    requested.sort();
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(
                &recipe.package_base,
                &recipe.commit,
                files,
                dependencies,
                &recipe.packages,
                requested
            ))
            .unwrap_or_default()
        )
    )
}
pub(crate) fn srcinfo(
    text: &str,
    base: &str,
    architecture: &str,
) -> AppResult<(BTreeMap<String, String>, Vec<AurDependency>)> {
    let mut pkgbase = None;
    let mut version = None;
    let mut release = None;
    let mut epoch = None;
    let mut names = BTreeSet::new();
    let mut deps = BTreeSet::new();
    let mut arches = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = line
            .split_once('=')
            .ok_or_else(|| invalid(".SRCINFO no contiene metadatos válidos"))?;
        let key = key.trim();
        let val = val.trim();
        match key {
            "pkgbase" => pkgbase = Some(val.to_string()),
            "pkgver" => version = Some(val.to_string()),
            "pkgrel" => release = Some(val.to_string()),
            "epoch" => epoch = Some(val.to_string()),
            "pkgname" => {
                if !package_name(val) {
                    return Err(invalid("Nombre de salida no válido en .SRCINFO"));
                }
                names.insert(val.to_string());
            }
            "arch" => {
                arches.insert(val.to_string());
            }
            _ => {
                for kind in ["depends", "makedepends", "checkdepends"] {
                    if key == kind || key == format!("{kind}_{architecture}") {
                        if val.is_empty() || val.contains(['\n', '\0', ' ', '\t']) {
                            return Err(invalid("Dependencia no válida en .SRCINFO"));
                        }
                        deps.insert((kind.to_string(), val.to_string()));
                    }
                }
            }
        }
    }
    if pkgbase.as_deref() != Some(base)
        || names.is_empty()
        || !arches.iter().any(|a| a == "any" || a == architecture)
    {
        return Err(invalid(
            "Base, arquitectura o paquetes de la receta no coinciden",
        ));
    }
    let version = version.ok_or_else(|| invalid("Falta pkgver en .SRCINFO"))?;
    let release = release.ok_or_else(|| invalid("Falta pkgrel en .SRCINFO"))?;
    let version = if let Some(epoch) = epoch {
        format!("{epoch}:{version}-{release}")
    } else {
        format!("{version}-{release}")
    };
    if version.len() > 200 || version.bytes().any(|c| c.is_ascii_whitespace() || c == 0) {
        return Err(invalid("Versión de receta no válida"));
    }
    Ok((
        names.into_iter().map(|n| (n, version.clone())).collect(),
        deps.into_iter()
            .map(|(kind, expression)| AurDependency {
                expression,
                kind: match kind.as_str() {
                    "makedepends" => "make",
                    "checkdepends" => "check",
                    _ => "runtime",
                }
                .into(),
            })
            .collect(),
    ))
}
pub(crate) fn read_recipe_files(repository: &Path, listed: &str) -> AppResult<Vec<RecipeFile>> {
    let mut result = vec![];
    let mut total = 0usize;
    for row in listed.split('\0').filter(|s| !s.is_empty()) {
        let (meta, path) = row
            .split_once('\t')
            .ok_or_else(|| invalid("Árbol Git no válido"))?;
        let fields = meta.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3
            || fields[1] != "blob"
            || !matches!(fields[0], "100644" | "100755")
            || !valid_relative(path)
        {
            return Err(invalid(
                "La receta contiene enlaces, submódulos o rutas fuera del checkout",
            ));
        }
        let fd = rustix::fs::openat2(
            rustix::fs::CWD,
            repository.join(path),
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
            rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|_| invalid("Archivo de receta enlazado"))?;
        let file = fs::File::from(fd);
        let m = file.metadata()?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.len() > 2 * 1024 * 1024
        {
            return Err(invalid(
                "Archivo de receta demasiado grande o con propietario inesperado",
            ));
        }
        let mut bytes = vec![];
        file.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        total += bytes.len();
        if bytes.len() > 2 * 1024 * 1024 || total > 16 * 1024 * 1024 || result.len() > 1000 {
            return Err(invalid("Receta demasiado grande"));
        }
        result.push(RecipeFile {
            path: path.into(),
            executable: fields[0] == "100755",
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
        });
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}
pub(crate) fn describe_diff(files: &[RecipeFile], old: Option<&AurRecipe>) -> String {
    let mut changes = vec![];
    for file in files {
        let before = old.and_then(|o| o.files.iter().find(|f| f.path == file.path));
        if before.is_some_and(|f| f.sha256 == file.sha256 && f.executable == file.executable) {
            continue;
        }
        let mut change = format!(
            "Archivo: {} · {} · SHA-256 {}\n",
            file.path,
            if file.executable {
                "ejecutable"
            } else {
                "datos"
            },
            file.sha256
        );
        if let Some(old) = before {
            change += &format!("--- anterior\n{}\n", String::from_utf8_lossy(&old.bytes));
        }
        if let Ok(text) = std::str::from_utf8(&file.bytes) {
            change += &format!("+++ revisado\n{text}\n");
        } else {
            change += "Archivo binario incluido en el digest; revisar su origen y hash.\n";
        }
        changes.push(change);
    }
    if let Some(old) = old {
        for file in &old.files {
            if !files.iter().any(|f| f.path == file.path) {
                changes.push(format!(
                    "Retirado: {} · SHA-256 {}\n",
                    file.path, file.sha256
                ));
            }
        }
    }
    changes.join("\n")
}
pub fn build_aur(recipe: &AurRecipe, ctx: &ExecutionContext) -> AppResult<Vec<BuiltArtifact>> {
    ensure_unprivileged(rustix::process::geteuid().as_raw())?;
    if recipe.digest != recipe_digest(recipe) {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Cambió el contenido de la receta revisada",
        ));
    }
    let base = ctx.paths.state.join("aur/recipes");
    let relative = recipe
        .repository
        .strip_prefix(&base)
        .map_err(|_| invalid("Checkout fuera del estado privado"))?;
    if relative.components().count() != 1
        || !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(invalid("Checkout AUR no válido"));
    }
    private_directory(&recipe.repository)?;
    let saved = fs::read(recipe.repository.join("review.json"))?;
    let saved: AurRecipe = serde_json::from_slice(&saved)?;
    if saved.digest != recipe.digest || recipe_digest(&saved) != recipe.digest {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "La receta no coincide con su revisión guardada",
        ));
    }
    let head = git(
        &ctx.provider.commands,
        Some(&recipe.repository),
        vec!["rev-parse".into(), "--verify".into(), "HEAD".into()],
    )?;
    if head.trim() != recipe.commit {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Cambió el commit revisado; solicita una receta nueva",
        ));
    }
    let cfg = AlpmConfig::load(&ctx.paths.pacman_conf, &ctx.provider.commands)?;
    let reader = AlpmReader::open(&cfg, &cfg.dbpath)?;
    for dependency in &recipe.dependencies {
        if !reader.satisfies(&dependency.expression)? {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                format!(
                    "Falta {} ({}) antes de compilar; preparar y revisar su dependencia de repositorio o receta AUR transitiva",
                    dependency.expression, dependency.kind
                ),
            ));
        }
    }
    let work = ctx
        .paths
        .state
        .join("aur/builds")
        .join(uuid::Uuid::new_v4().to_string());
    private_directory(&work)?;
    let checkout = work.join("recipe");
    let artifacts = work.join("artifacts");
    let source = work.join("sources");
    let build = work.join("build");
    let logs = work.join("logs");
    for dir in [&checkout, &artifacts, &source, &build, &logs] {
        private_directory(dir)?;
    }
    for file in &recipe.files {
        if !valid_relative(&file.path)
            || file.sha256 != format!("{:x}", Sha256::digest(&file.bytes))
        {
            return Err(invalid("Archivo de receta alterado"));
        }
        let dest = checkout.join(&file.path);
        private_directory(dest.parent().unwrap())?;
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if file.executable { 0o700 } else { 0o600 })
            .open(&dest)?;
        out.write_all(&file.bytes)?;
        out.sync_all()?;
    }
    ctx.emit.send(OperationEvent {
        operation_id: ctx.operation_id.clone(),
        seq: 1,
        phase: Phase::Building,
        message: format!(
            "Compilando {} en {} con receta revisada",
            recipe.package_base, recipe.commit
        ),
        progress: None,
        detail: None,
    })?;
    let mut spec = CommandSpec::new("/usr/bin/makepkg").args([
        "--cleanbuild".into(),
        "--check".into(),
        "--log".into(),
        "--nocolor".into(),
    ]);
    spec.cwd = Some(checkout);
    spec.timeout = Duration::from_secs(7200);
    spec.cancellation = ctx.cancellation.clone();
    for (key, path) in [
        ("PKGDEST", &artifacts),
        ("SRCDEST", &source),
        ("BUILDDIR", &build),
        ("LOGDEST", &logs),
        ("SRCPKGDEST", &work),
    ] {
        spec.env.insert(key.into(), path.to_string_lossy().into());
    }
    spec.env.insert("MAKEFLAGS".into(), "-j2".into());
    spec.env
        .insert("HOME".into(), ctx.paths.home.to_string_lossy().into());
    let outcome = ctx.provider.commands.run(&spec)?;
    for (name, content) in [
        ("makepkg.stdout.log", &outcome.stdout),
        ("makepkg.stderr.log", &outcome.stderr),
    ] {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(logs.join(name))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    // No retries, automatic installation, dependency removal, or privilege escalation.
    if outcome.code != 0 {
        return Err(AppError::new(
            ErrorCode::TransactionFailed,
            format!(
                "Falló makepkg para {} (código {}); los cambios de sistema anteriores se conservan. Registro: {}",
                recipe.package_base,
                outcome.code,
                logs.display()
            ),
        ));
    }
    let mut result = vec![];
    let mut produced = BTreeSet::new();
    for entry in fs::read_dir(&artifacts)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(".sig") {
            continue;
        }
        if !name.contains(".pkg.tar.") {
            return Err(invalid("Salida inesperada de makepkg"));
        }
        let m = fs::symlink_metadata(&path)?;
        if !m.is_file()
            || m.file_type().is_symlink()
            || m.nlink() != 1
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.len() > 2 * 1024 * 1024 * 1024
        {
            return Err(invalid("Artefacto de compilación no válido"));
        }
        let info = reader.archive(&path, 0)?;
        if !recipe.packages.contains_key(&info.name) {
            return Err(invalid(
                "La compilación produjo un paquete ausente de .SRCINFO revisado",
            ));
        }
        if !produced.insert(info.name.clone()) {
            return Err(invalid(
                "La compilación produjo dos artefactos para el mismo paquete",
            ));
        }
        if recipe.requested.contains(&info.name) {
            let mut file = fs::File::open(&path)?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            result.push(BuiltArtifact {
                name: info.name,
                version: info.version,
                path,
                sha256: format!("{:x}", hash.finalize()),
            });
        }
    }
    if recipe
        .requested
        .iter()
        .any(|n| !result.iter().any(|a| &a.name == n))
    {
        return Err(invalid("Falta un artefacto aprobado de la compilación"));
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct ResolvedDependencies {
    pub repository: Vec<String>,
    pub aur_bases: Vec<String>,
}
pub fn resolve_dependencies(
    recipe: &AurRecipe,
    snapshot: &crate::providers::arch::RepoSnapshot,
    reviewed: &BTreeMap<String, AurRecipe>,
    policy: &crate::policy::Policy,
) -> AppResult<ResolvedDependencies> {
    struct ResolutionState {
        stack: BTreeSet<String>,
        done: BTreeSet<String>,
        repository: BTreeSet<String>,
        order: Vec<String>,
    }
    fn visit(
        recipe: &AurRecipe,
        snapshot: &crate::providers::arch::RepoSnapshot,
        reviewed: &BTreeMap<String, AurRecipe>,
        policy: &crate::policy::Policy,
        state: &mut ResolutionState,
    ) -> AppResult<()> {
        if state.done.contains(&recipe.package_base) {
            return Ok(());
        }
        if !state.stack.insert(recipe.package_base.clone()) {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "Las dependencias AUR forman un ciclo; requieren revisión",
            ));
        }
        if recipe.digest != recipe_digest(recipe) {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "Cambió una receta transitiva",
            ));
        }
        for dep in &recipe.dependencies {
            if snapshot.installed.iter().any(|p| {
                crate::planner::satisfies(&dep.expression, &p.name, &p.version, &p.provides)
            }) {
                continue;
            }
            let candidates = snapshot
                .packages
                .iter()
                .filter(|p| {
                    snapshot
                        .preferred(&p.name)
                        .is_some_and(|first| first.repository == p.repository)
                        && crate::planner::satisfies(
                            &dep.expression,
                            &p.name,
                            &p.version,
                            &p.provides,
                        )
                })
                .collect::<Vec<_>>();
            if candidates.len() > 1 {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    format!(
                        "Varios proveedores para {}; requiere seleccionar un proveedor",
                        dep.expression
                    ),
                ));
            }
            if let Some(package) = candidates.first() {
                crate::planner::protect_package(package, snapshot, policy)?;
                state.repository.insert(package.name.clone());
                continue;
            }
            let candidates = reviewed
                .values()
                .filter(|r| {
                    r.packages.iter().any(|(name, version)| {
                        crate::planner::satisfies(&dep.expression, name, version, &[])
                    })
                })
                .collect::<Vec<_>>();
            if candidates.len() != 1 {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    format!(
                        "{} requiere una receta AUR transitiva revisada",
                        dep.expression
                    ),
                ));
            }
            let dependency = candidates[0];
            if dependency
                .packages
                .keys()
                .any(|n| policy.protected_package(n))
            {
                return Err(AppError::new(
                    ErrorCode::CoverageNeedsReview,
                    "Una dependencia AUR toca la suite excluida",
                ));
            }
            visit(dependency, snapshot, reviewed, policy, state)?;
        }
        state.stack.remove(&recipe.package_base);
        state.done.insert(recipe.package_base.clone());
        state.order.push(recipe.package_base.clone());
        Ok(())
    }
    let mut state = ResolutionState {
        stack: BTreeSet::new(),
        done: BTreeSet::new(),
        repository: BTreeSet::new(),
        order: vec![],
    };
    visit(recipe, snapshot, reviewed, policy, &mut state)?;
    state.order.retain(|name| name != &recipe.package_base);
    Ok(ResolvedDependencies {
        repository: state.repository.into_iter().collect(),
        aur_bases: state.order,
    })
}

pub fn record_approved_recipe(recipe: &AurRecipe, paths: &crate::paths::Paths) -> AppResult<()> {
    if !package_name(&recipe.package_base) || recipe.digest != recipe_digest(recipe) {
        return Err(invalid("Receta aprobada inválida"));
    }
    crate::store::atomic_json(
        &paths
            .state
            .join("aur/reviews")
            .join(format!("{}.json", recipe.package_base)),
        recipe,
    )
}
pub fn attach_artifacts(
    plan: &UpdatePlan,
    recipe: &AurRecipe,
    artifacts: &[BuiltArtifact],
    now: u64,
) -> AppResult<UpdatePlan> {
    let mut plan = plan.clone();
    let mut matched = false;
    for step in &mut plan.steps {
        if step.kind != StepKind::AurBuild
            || step.aur_recipe.as_ref().is_none_or(|r| {
                r.digest != recipe.digest
                    || r.commit != recipe.commit
                    || r.package_base != recipe.package_base
            })
        {
            continue;
        }
        if artifacts.len() != recipe.requested.len()
            || artifacts.iter().any(|a| {
                !recipe.requested.contains(&a.name)
                    || a.sha256.len() != 64
                    || !a.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            })
        {
            return Err(invalid(
                "Los artefactos no coinciden con los destinos de la receta revisada",
            ));
        }
        let mut names = BTreeSet::new();
        for artifact in artifacts {
            if !names.insert(&artifact.name) {
                return Err(invalid("Artefactos duplicados"));
            }
            let change = step
                .package_changes
                .iter_mut()
                .find(|p| p.name == artifact.name)
                .ok_or_else(|| invalid("Falta el destino de instalación"))?;
            if change.new_version != artifact.version {
                plan.notices.push(format!(
                    "AUR {}: la compilación produjo {} en lugar de {}; requiere revisar el plan",
                    artifact.name, artifact.version, change.new_version
                ));
                change.new_version = artifact.version.clone();
            }
        }
        step.built_artifacts = artifacts.to_vec();
        matched = true;
    }
    if !matched {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "La receta no pertenece al plan aprobado",
        ));
    }
    plan.notices.push(
        "AUR: revisar los paquetes generados y sus SHA-256 antes de la instalación administrativa"
            .into(),
    );
    plan.created_at = now;
    plan.expires_at = now.saturating_add(900);
    plan.digest = crate::planner::plan_digest(&plan);
    Ok(plan)
}

pub fn preserve_build_failure(
    mut system_results: Vec<TargetResult>,
    targets: &[Target],
    error: &AppError,
) -> Vec<TargetResult> {
    system_results.extend(targets.iter().map(|target| TargetResult {
        target_id: target.id.clone(),
        outcome: Outcome::Failed,
        old_version: target.installed.clone(),
        new_version: None,
        error: Some(error.clone()),
    }));
    system_results
}
