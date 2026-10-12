use super::*;
use crate::io::http::DownloadPolicy;
pub struct AurProvider;
impl Provider for AurProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let package = match &target.locator {
            Locator::Aur { package, .. } => package,
            _ => return Err(AppError::new(ErrorCode::InvalidInput, "Destino no es AUR")),
        };
        if package.is_empty()
            || !package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Nombre de paquete inválido",
            ));
        }
        let mut url = reqwest::Url::parse("https://aur.archlinux.org/rpc/v5/info").unwrap();
        url.query_pairs_mut().append_pair("arg[]", package);
        let data = ctx.http.json(
            url.as_str(),
            &DownloadPolicy::for_hosts(&["aur.archlinux.org"], 2_097_152),
        )?;
        let row = data
            .get("results")
            .and_then(|r| r.as_array())
            .and_then(|r| r.iter().find(|r| r["Name"] == package.as_str()))
            .ok_or_else(|| AppError::new(ErrorCode::Unsupported, "El paquete no aparece en AUR"))?;
        let version = row["Version"]
            .as_str()
            .ok_or_else(|| AppError::new(ErrorCode::NetworkError, "AUR no informó una versión"))?;
        let available = Version::arch(version);
        let mut check = check_base(
            target,
            ctx,
            version_state(target.installed.as_ref(), &available)?,
        );
        check.available = Some(available);
        check.source = Some(format!("https://aur.archlinux.org/packages/{package}"));
        if !row["OutOfDate"].is_null() {
            check
                .details
                .push("La receta AUR está marcada como desactualizada".into());
        }
        Ok(check)
    }
}

pub fn prepare_aur(
    target: &Target,
    ctx: &ProviderContext,
) -> AppResult<crate::execution::aur::AurRecipe> {
    use crate::execution::aur::{
        AurRecipe, describe_diff, git, package_name, private_directory, read_recipe_files,
        recipe_digest, srcinfo,
    };
    use std::{fs, io::Write, os::unix::fs::OpenOptionsExt};
    crate::execution::aur::ensure_unprivileged(rustix::process::geteuid().as_raw())?;
    let Locator::Aur { package, base } = &target.locator else {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "El destino no es una receta AUR",
        ));
    };
    if !package_name(package) || !package_name(base) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Nombre AUR no válido",
        ));
    }
    let parent = ctx.paths.state.join("aur/recipes");
    private_directory(&parent)?;
    let repository = parent.join(uuid::Uuid::new_v4().to_string());
    private_directory(&repository)?;
    git(
        &ctx.commands,
        None,
        vec![
            "clone".into(),
            "--no-checkout".into(),
            "--no-tags".into(),
            "--depth=1".into(),
            "--".into(),
            format!("https://aur.archlinux.org/{base}.git"),
            repository.to_string_lossy().into(),
        ],
    )?;
    let commit = git(
        &ctx.commands,
        Some(&repository),
        vec!["rev-parse".into(), "--verify".into(), "HEAD".into()],
    )?
    .trim()
    .to_owned();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "AUR no devolvió un commit verificable",
        ));
    }
    let listed = git(
        &ctx.commands,
        Some(&repository),
        vec![
            "ls-tree".into(),
            "-rz".into(),
            "--full-tree".into(),
            commit.clone(),
        ],
    )?;
    // Reject links/submodules before checkout, which could otherwise follow external content.
    for row in listed.split('\0').filter(|r| !r.is_empty()) {
        let (meta, path) = row
            .split_once('\t')
            .ok_or_else(|| AppError::new(ErrorCode::UnsafeArtifact, "Árbol Git no válido"))?;
        if !matches!(meta.split_whitespace().next(), Some("100644" | "100755"))
            || std::path::Path::new(path)
                .components()
                .any(|c| !matches!(c,std::path::Component::Normal(n)if n!=".git"))
        {
            return Err(AppError::new(
                ErrorCode::UnsafeArtifact,
                "La receta contiene enlaces, submódulos o rutas no revisables",
            ));
        }
    }
    git(
        &ctx.commands,
        Some(&repository),
        vec!["checkout".into(), "--detach".into(), commit.clone()],
    )?;
    let files = read_recipe_files(&repository, &listed)?;
    let info = files.iter().find(|f| f.path == ".SRCINFO").ok_or_else(|| {
        AppError::new(
            ErrorCode::Unsupported,
            "La receta no contiene .SRCINFO; no se ejecuta PKGBUILD para generarlo",
        )
    })?;
    let info = std::str::from_utf8(&info.bytes)
        .map_err(|_| AppError::new(ErrorCode::UnsafeArtifact, ".SRCINFO no es texto válido"))?;
    let cfg = crate::alpm::AlpmConfig::load(&ctx.paths.pacman_conf, &ctx.commands)?;
    let (packages, dependencies) = srcinfo(info, base, &cfg.architecture)?;
    if !packages.contains_key(package) {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "El paquete ya no está declarado en la receta de su base",
        ));
    }
    let review_path = ctx
        .paths
        .state
        .join("aur/reviews")
        .join(format!("{base}.json"));
    let old = crate::store::read_private(&review_path, 16 * 1024 * 1024)?
        .and_then(|bytes| serde_json::from_slice::<AurRecipe>(&bytes).ok());
    let diff = describe_diff(&files, old.as_ref());
    let mut recipe = AurRecipe {
        package_base: base.clone(),
        commit,
        files,
        diff,
        dependencies,
        digest: String::new(),
        requested: vec![package.clone()],
        packages,
        repository,
    };
    recipe.digest = recipe_digest(&recipe);
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(recipe.repository.join("review.json"))?;
    out.write_all(&serde_json::to_vec(&recipe)?)?;
    out.sync_all()?;
    Ok(recipe)
}

pub fn package_base(package: &str, ctx: &ProviderContext) -> AppResult<String> {
    if !crate::execution::aur::package_name(package) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Paquete AUR inválido",
        ));
    }
    let mut url = reqwest::Url::parse("https://aur.archlinux.org/rpc/v5/info").unwrap();
    url.query_pairs_mut().append_pair("arg[]", package);
    let data = ctx.http.json(
        url.as_str(),
        &DownloadPolicy::for_hosts(&["aur.archlinux.org"], 2_097_152),
    )?;
    let base = data["results"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["Name"] == package))
        .and_then(|row| row["PackageBase"].as_str())
        .filter(|b| crate::execution::aur::package_name(b))
        .ok_or_else(|| AppError::new(ErrorCode::Unsupported, "Base AUR no verificable"))?;
    Ok(base.into())
}
