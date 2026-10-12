use crate::{
    alpm::{AvailablePackage, arch_compare},
    domain::*,
    policy::Policy,
    providers::arch::RepoSnapshot,
};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn hash(value: &impl serde::Serialize) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(value).expect("plan DTOs contain JSON-serializable data")
        )
    )
}
pub fn inventory_digest(report: &Report, selection: &[AppId]) -> AppResult<String> {
    let mut apps = report.apps.clone();
    apps.sort_by(|a, b| a.id.cmp(&b.id));
    for app in &mut apps {
        app.components.sort_by(|a, b| a.id.cmp(&b.id));
        app.desktop_ids.sort();
    }
    let mut checks = report.checks.clone();
    checks.sort_by(|a, b| a.target_id.cmp(&b.target_id));
    for check in &mut checks {
        check.checked_at = 0;
        check.published = None;
        check.details.clear();
    }
    let mut selected = selection.to_vec();
    selected.sort();
    selected.dedup();
    let mut effective = report.selected.clone();
    effective.sort();
    effective.dedup();
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(apps, checks, selected, effective))?)
    ))
}
pub fn repository_digest(snapshot: &RepoSnapshot) -> String {
    let mut packages = snapshot.packages.clone();
    packages.sort_by(|a, b| (&a.repository, &a.name).cmp(&(&b.repository, &b.name)));
    let mut installed = snapshot.installed.clone();
    installed.sort_by(|a, b| a.name.cmp(&b.name));
    hash(&(
        &snapshot.repositories,
        &snapshot.config_digest,
        packages,
        installed,
    ))
}
pub fn plan_digest(plan: &UpdatePlan) -> String {
    let mut canonical = plan.clone();
    canonical.id = PlanId::default();
    canonical.created_at = 0;
    canonical.expires_at = 0;
    canonical.digest.clear();
    canonical.selected.sort();
    for step in &mut canonical.steps {
        step.targets.sort();
        step.reopen.sort();
        step.package_changes.sort_by(|a, b| {
            (&a.name, &a.new_version, &a.repository).cmp(&(&b.name, &b.new_version, &b.repository))
        });
        step.downloads.sort_by(|a, b| {
            (&a.url, &a.expected_sha256, a.bytes).cmp(&(&b.url, &b.expected_sha256, b.bytes))
        });
        step.flatpak_changes
            .sort_by(|a, b| (&a.installation, &a.reference).cmp(&(&b.installation, &b.reference)));
    }
    // Step sequence is execution order; changing it is a material change.
    hash(&canonical)
}
pub fn validate_plan(
    plan: &UpdatePlan,
    report: &Report,
    policy: &Policy,
    now: u64,
) -> AppResult<()> {
    if !policy.coverage.verified() || !report.coverage.verified() {
        return Err(AppError::new(
            ErrorCode::CoverageNeedsReview,
            "Revisar la cobertura de updater.py antes de actualizar",
        ));
    }
    if now < plan.created_at
        || now >= plan.expires_at
        || plan.expires_at.saturating_sub(plan.created_at) > 900
    {
        return Err(AppError::new(
            ErrorCode::PlanExpired,
            "El plan venció; prepara uno nuevo",
        ));
    }
    if plan.digest != plan_digest(plan)
        || plan.inventory_digest != inventory_digest(report, &plan.selected)?
        || plan.policy_digest != policy.digest()
    {
        return Err(AppError::new(
            ErrorCode::PlanChanged,
            "Cambió el inventario, la selección o el alcance aprobado",
        ));
    }
    for id in &plan.selected {
        let app = report.apps.iter().find(|a| &a.id == id).ok_or_else(|| {
            AppError::new(ErrorCode::PlanChanged, "La aplicación ya no está instalada")
        })?;
        if policy.excluded(app) {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                "Aplicación cubierta por updater.py",
            ));
        }
    }
    Ok(())
}
pub fn empty_step(kind: StepKind, targets: Vec<TargetId>) -> PlanStep {
    PlanStep {
        built_artifacts: vec![],
        build_dependencies: vec![],
        kind,
        targets,
        flatpak_changes: vec![],
        package_changes: vec![],
        downloads: vec![],
        requires_auth: false,
        reopen: vec![],
        aur_recipe: None,
    }
}
fn dependency_parts(dep: &str) -> (&str, &str, &str) {
    if let Some(i) = dep.find(['<', '>', '=']) {
        let (name, tail) = dep.split_at(i);
        let n = tail
            .bytes()
            .take_while(|c| matches!(c, b'<' | b'>' | b'='))
            .count();
        (name, &tail[..n], &tail[n..])
    } else {
        (dep, "", "")
    }
}
fn matches_version(value: &str, operator: &str, wanted: &str) -> bool {
    if operator.is_empty() {
        return true;
    }
    let Ok(c) = arch_compare(value, wanted) else {
        return false;
    };
    match operator {
        "=" => c == Ordering::Equal,
        ">=" => c != Ordering::Less,
        "<=" => c != Ordering::Greater,
        ">" => c == Ordering::Greater,
        "<" => c == Ordering::Less,
        _ => false,
    }
}
pub(crate) fn satisfies(dep: &str, name: &str, version: &str, provides: &[String]) -> bool {
    let (wanted, op, v) = dependency_parts(dep);
    if wanted == name && matches_version(version, op, v) {
        return true;
    }
    provides.iter().any(|provide| {
        let (p, po, pv) = dependency_parts(provide);
        p == wanted && (op.is_empty() || po == "=" && matches_version(pv, op, v))
    })
}
pub(crate) fn protect_package(
    package: &AvailablePackage,
    snapshot: &RepoSnapshot,
    policy: &Policy,
) -> AppResult<()> {
    let files = package.files.iter().chain(
        snapshot
            .installed
            .iter()
            .filter(|p| p.name == package.name)
            .flat_map(|p| &p.files),
    );
    if policy.protected_package(&package.name)
        || files
            .into_iter()
            .any(|f| policy.protected_path(&Path::new("/").join(f)))
    {
        return Err(AppError::new(
            ErrorCode::CoverageNeedsReview,
            format!(
                "El alcance completo incluye {} y toca una herramienta cubierta por updater.py; no se permite omitirla ni actualizar parcialmente",
                package.name
            ),
        ));
    }
    Ok(())
}
pub fn system_scope(
    snapshot: &RepoSnapshot,
    selected_packages: &BTreeSet<String>,
    policy: &Policy,
    build_dependencies: &BTreeSet<String>,
) -> AppResult<Vec<PackageChange>> {
    let mut changes: BTreeMap<String, &AvailablePackage> = BTreeMap::new();
    for installed in &snapshot.installed {
        if let Some(next) = snapshot.preferred(&installed.name)
            && arch_compare(&next.version, &installed.version)? == Ordering::Greater
        {
            if installed.ignored {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    format!(
                        "{} está retenido por IgnorePkg/IgnoreGroup; requiere revisar coherencia antes de actualizar el sistema",
                        installed.name
                    ),
                ));
            }
            protect_package(next, snapshot, policy)?;
            changes.insert(next.name.clone(), next);
        }
    }
    for name in build_dependencies {
        let package = snapshot.preferred(name).ok_or_else(|| {
            AppError::new(
                ErrorCode::PlanChanged,
                "Falta una dependencia de compilación en los repositorios",
            )
        })?;
        protect_package(package, snapshot, policy)?;
        if snapshot
            .installed
            .iter()
            .find(|p| &p.name == name)
            .is_none_or(|p| p.version != package.version)
        {
            changes.insert(name.clone(), package);
        }
    }
    let mut pending = changes.keys().cloned().collect::<BTreeSet<_>>();
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop_first() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let package = changes[&name];
        for dep in &package.depends {
            let changed_satisfies = changes
                .values()
                .any(|p| satisfies(dep, &p.name, &p.version, &p.provides));
            let installed_satisfies = snapshot
                .installed
                .iter()
                .filter(|p| !changes.contains_key(&p.name))
                .any(|p| satisfies(dep, &p.name, &p.version, &p.provides));
            if changed_satisfies || installed_satisfies {
                continue;
            }
            let (depname, _, _) = dependency_parts(dep);
            let candidate = if let Some(p) = snapshot
                .preferred(depname)
                .filter(|p| satisfies(dep, &p.name, &p.version, &p.provides))
            {
                p
            } else {
                let mut options = BTreeMap::new();
                for p in &snapshot.packages {
                    if snapshot
                        .preferred(&p.name)
                        .is_some_and(|best| best.repository == p.repository)
                        && satisfies(dep, &p.name, &p.version, &p.provides)
                    {
                        options.insert(p.name.clone(), p);
                    }
                }
                if options.len() != 1 {
                    return Err(AppError::new(
                        ErrorCode::PlanChanged,
                        format!("La dependencia {dep} necesita una revisión de proveedor"),
                    ));
                }
                *options.values().next().unwrap()
            };
            if let Some(installed) = snapshot.installed.iter().find(|p| p.name == candidate.name)
                && (installed.ignored
                    || arch_compare(&candidate.version, &installed.version)? == Ordering::Less)
            {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    format!(
                        "La dependencia {} está retenida o requeriría degradación",
                        candidate.name
                    ),
                ));
            }
            protect_package(candidate, snapshot, policy)?;
            changes.insert(candidate.name.clone(), candidate);
            pending.insert(candidate.name.clone());
        }
    }
    // Check reverse dependencies, including foreign/local packages, against the complete candidate state.
    for installed in snapshot
        .installed
        .iter()
        .filter(|p| !changes.contains_key(&p.name))
    {
        for dep in &installed.depends {
            if !changes
                .values()
                .any(|p| satisfies(dep, &p.name, &p.version, &p.provides))
                && !snapshot
                    .installed
                    .iter()
                    .filter(|p| !changes.contains_key(&p.name))
                    .any(|p| satisfies(dep, &p.name, &p.version, &p.provides))
            {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    format!(
                        "{} requiere {dep}; revisar la coherencia de los paquetes locales",
                        installed.name
                    ),
                ));
            }
        }
    }
    Ok(changes
        .values()
        .map(|p| {
            let installed = snapshot.installed.iter().find(|i| i.name == p.name);
            PackageChange {
                action: if installed.is_some() {
                    PackageAction::Upgrade
                } else {
                    PackageAction::Install
                },
                name: p.name.clone(),
                old_version: installed.map(|i| i.version.clone()),
                new_version: p.version.clone(),
                repository: Some(p.repository.clone()),
                reason: if build_dependencies.contains(&p.name) {
                    "build_dependency"
                } else if selected_packages.contains(&p.name) {
                    "app"
                } else if installed.is_some() {
                    "system"
                } else {
                    "dependency"
                }
                .into(),
                download_bytes: Some(p.download_bytes),
            }
        })
        .collect())
}
pub fn build_plan(
    report: &Report,
    selection: &[AppId],
    snapshot: &RepoSnapshot,
    policy: &Policy,
    now: u64,
) -> AppResult<UpdatePlan> {
    build_plan_with_recipes(report, selection, snapshot, policy, now, &BTreeMap::new())
}
pub fn build_plan_with_recipes(
    report: &Report,
    selection: &[AppId],
    snapshot: &RepoSnapshot,
    policy: &Policy,
    now: u64,
    recipes: &BTreeMap<TargetId, crate::execution::aur::AurRecipe>,
) -> AppResult<UpdatePlan> {
    if !policy.coverage.verified() || !report.coverage.verified() {
        return Err(AppError::new(
            ErrorCode::CoverageNeedsReview,
            "Revisar la cobertura de updater.py antes de actualizar",
        ));
    }
    let mut selected = selection.to_vec();
    selected.sort();
    selected.dedup();
    let mut steps = vec![];
    let mut arch_targets = vec![];
    let mut selected_packages = BTreeSet::new();
    let mut requires_coherence = false;
    let mut build_dependencies = BTreeSet::new();
    let reviewed = recipes
        .values()
        .map(|r| (r.package_base.clone(), r.clone()))
        .collect::<BTreeMap<_, _>>();
    for id in &selected {
        let app = report.apps.iter().find(|a| &a.id == id).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "Aplicación ausente del inventario")
        })?;
        if policy.excluded(app) {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                "Aplicación cubierta por updater.py",
            ));
        }
        for target in &app.components {
            if matches!(target.locator, Locator::Web { .. }) {
                continue;
            }
            if matches!(target.locator, Locator::Unknown { .. }) {
                return Err(AppError::new(
                    ErrorCode::Unsupported,
                    format!("No hay un proveedor seguro para {}", app.name),
                ));
            }
            let check = report.checks.iter().find(|c| c.target_id == target.id);
            let Some(check) = check.filter(|c| c.state == CheckState::UpdateAvailable && !c.stale)
            else {
                continue;
            };
            let available = check.available.as_ref().ok_or_else(|| {
                AppError::new(ErrorCode::PlanChanged, "Versión disponible desconocida")
            })?;
            match &target.locator {
                Locator::Repo { package } => {
                    if snapshot.preferred(package).is_none() {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Cambió el repositorio del paquete",
                        ));
                    }
                    let installed = snapshot
                        .installed
                        .iter()
                        .find(|p| &p.name == package)
                        .ok_or_else(|| {
                            AppError::new(ErrorCode::PlanChanged, "El paquete ya no está instalado")
                        })?;
                    if check
                        .installed
                        .as_ref()
                        .is_none_or(|v| v.raw != installed.version)
                    {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "Cambió la versión instalada; vuelve a comprobar",
                        ));
                    }
                    requires_coherence = true;
                    selected_packages.insert(package.clone());
                    arch_targets.push(target.id.clone());
                }
                Locator::Aur { package, base } => {
                    let recipe = recipes.get(&target.id).ok_or_else(|| {
                        AppError::new(
                            ErrorCode::Unsupported,
                            "La receta AUR debe prepararse y revisarse antes de construir el plan",
                        )
                    })?;
                    if &recipe.package_base != base
                        || !recipe.requested.contains(package)
                        || recipe.digest != crate::execution::aur::recipe_digest(recipe)
                    {
                        return Err(AppError::new(
                            ErrorCode::PlanChanged,
                            "La receta no coincide con el destino AUR",
                        ));
                    }
                    let deps = crate::execution::aur::resolve_dependencies(
                        recipe, snapshot, &reviewed, policy,
                    )?;
                    build_dependencies.extend(deps.repository);
                    for base in deps.aur_bases {
                        if steps.iter().any(|s: &PlanStep| {
                            s.aur_recipe
                                .as_ref()
                                .is_some_and(|r| r.package_base == base)
                        }) {
                            continue;
                        }
                        let dependency = reviewed.get(&base).ok_or_else(|| {
                            AppError::new(
                                ErrorCode::Unsupported,
                                "Falta la receta de una dependencia transitiva",
                            )
                        })?;
                        let mut dependency_step = empty_step(
                            StepKind::AurBuild,
                            dependency
                                .requested
                                .iter()
                                .map(|name| TargetId(format!("aur:{name}")))
                                .collect(),
                        );
                        dependency_step.aur_recipe = Some(AurRecipeRef {
                            package_base: base,
                            commit: dependency.commit.clone(),
                            digest: dependency.digest.clone(),
                        });
                        for package in &dependency.requested {
                            let version = dependency.packages.get(package).ok_or_else(|| {
                                AppError::new(
                                    ErrorCode::PlanChanged,
                                    "La dependencia no declara su paquete",
                                )
                            })?;
                            dependency_step.package_changes.push(PackageChange {
                                action: PackageAction::Install,
                                name: package.clone(),
                                old_version: None,
                                new_version: version.clone(),
                                repository: Some("AUR".into()),
                                reason: "build_dependency".into(),
                                download_bytes: None,
                            });
                        }
                        dependency_step.build_dependencies = dependency
                            .dependencies
                            .iter()
                            .map(|d| format!("{}: {}", d.kind, d.expression))
                            .collect();
                        dependency_step.requires_auth = true;
                        steps.push(dependency_step);
                    }
                    let mut step = empty_step(StepKind::AurBuild, vec![target.id.clone()]);
                    step.aur_recipe = Some(AurRecipeRef {
                        package_base: base.clone(),
                        commit: recipe.commit.clone(),
                        digest: recipe.digest.clone(),
                    });
                    step.package_changes.push(PackageChange {
                        action: PackageAction::Upgrade,
                        name: package.clone(),
                        old_version: target.installed.as_ref().map(|v| v.raw.clone()),
                        new_version: recipe.packages.get(package).cloned().ok_or_else(|| {
                            AppError::new(ErrorCode::PlanChanged, "Falta la versión de salida AUR")
                        })?,
                        repository: Some("AUR".into()),
                        reason: "app".into(),
                        download_bytes: None,
                    });
                    step.build_dependencies = recipe
                        .dependencies
                        .iter()
                        .map(|d| format!("{}: {}", d.kind, d.expression))
                        .collect();
                    step.requires_auth = true;
                    steps.push(step);
                    requires_coherence = true;
                }
                Locator::Flatpak { installation, .. } => {
                    let mut step = empty_step(StepKind::FlatpakUpdate, vec![target.id.clone()]);
                    step.flatpak_changes =
                        crate::execution::flatpak::prepare_flatpak(target, &available.raw)?;
                    step.requires_auth = matches!(installation, FlatpakInstall::System { .. });
                    if !step.flatpak_changes.is_empty() {
                        steps.push(step);
                    }
                }
                Locator::TelegramLocal { .. } => {
                    if available.kind != VersionKind::Product
                        || !available
                            .raw
                            .chars()
                            .all(|c| c.is_ascii_digit() || c == '.')
                    {
                        return Err(AppError::new(
                            ErrorCode::UnsafeArtifact,
                            "Versión Telegram inválida",
                        ));
                    }
                    let mut step = empty_step(StepKind::TelegramReplace, vec![target.id.clone()]);
                    step.downloads.push(Download {
                        url: format!(
                            "https://td.telegram.org/linux-x64/td-setup-linux-x64-{}.tar.xz",
                            available.raw
                        ),
                        expected_sha256: None,
                        bytes: None,
                    });
                    steps.push(step);
                }
                Locator::SteamClient { channel, .. } => {
                    let mut step = empty_step(StepKind::SteamUpdate, vec![target.id.clone()]);
                    step.downloads.push(Download {
                        url: crate::providers::steam::manifest_url(channel)?,
                        expected_sha256: None,
                        bytes: None,
                    });
                    steps.push(step);
                }
                Locator::Web { .. } | Locator::Unknown { .. } => unreachable!(),
            }
        }
    }
    let mut notices = vec![];
    if requires_coherence {
        let changes = system_scope(snapshot, &selected_packages, policy, &build_dependencies)?;
        if !changes.is_empty() || steps.iter().any(|s| s.kind == StepKind::AurBuild) {
            let mut step = empty_step(StepKind::ArchFull, arch_targets);
            step.package_changes = changes;
            step.build_dependencies = build_dependencies.iter().cloned().collect();
            step.requires_auth = true;
            steps.insert(0, step);
            notices.push("Arch: actualización completa de paquetes, incluidas dependencias y cambios de sistema".into());
            notices.push("Snapshot: se comprobará Snapper antes de instalar; si no está disponible o falla, se solicitará una decisión".into());
            if steps[0].package_changes.iter().any(|p| p.name == "omarchy") {
                notices.push("Omarchy: sus migraciones deben revisarse antes de confirmar; se detendrá si afectan la suite de IA excluida".into());
            }
        }
    }
    if steps.is_empty() {
        notices.push("No hay actualizaciones instalables para esta selección".into());
    }
    let mut plan = UpdatePlan {
        decisions: vec![],
        id: uuid::Uuid::new_v4().to_string().into(),
        created_at: now,
        expires_at: now.saturating_add(900),
        inventory_digest: inventory_digest(report, &selected)?,
        policy_digest: policy.digest(),
        repository_digest: repository_digest(snapshot),
        selected,
        notices,
        steps,
        digest: String::new(),
    };
    plan.digest = plan_digest(&plan);
    Ok(plan)
}
