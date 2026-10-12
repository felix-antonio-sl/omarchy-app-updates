use crate::{
    domain::*,
    paths::Paths,
    policy::Policy,
    providers::{self, Provider, ProviderContext},
};
use std::sync::Mutex;

/// At most two transient check workers; no daemon or automatic update timer.
pub fn refresh_with_provider(
    report: &Report,
    policy: &Policy,
    ctx: &ProviderContext,
    provider: &dyn Provider,
) -> AppResult<Report> {
    let mut report = report.clone();
    report.apps = policy.visible(&report.apps);
    let tasks = report
        .apps
        .iter()
        .flat_map(|a| a.components.clone())
        .collect::<Vec<_>>();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = Mutex::new(vec![None; tasks.len()]);
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let tasks = &tasks;
            let next = &next;
            let results = &results;
            scope.spawn(move || {
                loop {
                    let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(target) = tasks.get(n) else {
                        break;
                    };
                    let checked = provider.check(target, ctx);
                    results.lock().unwrap()[n] = Some(checked);
                }
            });
        }
    });
    let previous = report.checks.clone();
    report.checks = tasks
        .iter()
        .zip(results.into_inner().unwrap())
        .map(|(t, r)| match r.expect("every check worker joined") {
            Ok(c) => c,
            Err(e) => {
                let mut c = previous
                    .iter()
                    .find(|c| c.target_id == t.id)
                    .cloned()
                    .unwrap_or_else(|| providers::check_base(t, ctx, CheckState::Unknown));
                c.installed = t.installed.clone().or(c.installed);
                c.stale = true;
                c.details
                    .push(format!("No se pudo comprobar la fuente: {}", e.message));
                if c.details.len() > 20 {
                    c.details.drain(..c.details.len() - 20);
                }
                c
            }
        })
        .collect();
    for app in &mut report.apps {
        for target in &mut app.components {
            if let Some(check) = report.checks.iter().find(|c| c.target_id == target.id) {
                target.installed = check.installed.clone();
            }
        }
    }
    report.checked_at = Some(ctx.now);
    Ok(report)
}
struct Checks {
    arch: providers::arch::ArchProvider,
}
impl Provider for Checks {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let mut check = match &target.locator {
            Locator::Repo { .. } => self.arch.check(target, ctx),
            Locator::Aur { package, .. } => {
                if self
                    .arch
                    .snapshot()
                    .ok()
                    .is_some_and(|s| s.preferred(package).is_some())
                {
                    let mut c = providers::check_base(target, ctx, CheckState::Unknown);
                    c.details.push("Cambió el origen: el paquete ahora está en un repositorio configurado; revisa el proveedor antes de instalar".into());
                    Ok(c)
                } else {
                    providers::aur::AurProvider.check(target, ctx)
                }
            }
            Locator::Flatpak { .. } => providers::flatpak::FlatpakProvider.check(target, ctx),
            Locator::TelegramLocal { .. } => {
                providers::telegram::TelegramProvider.check(target, ctx)
            }
            Locator::SteamClient { .. } => providers::steam::SteamProvider.check(target, ctx),
            Locator::Web { .. } => providers::web::WebProvider.check(target, ctx),
            Locator::Unknown { .. } => {
                Ok(providers::check_base(target, ctx, CheckState::Unsupported))
            }
        }?;
        if matches!(target.locator, Locator::Repo { .. } | Locator::Aur { .. }) {
            providers::releases::annotate_published(&mut check, target, ctx);
        }
        Ok(check)
    }
}
pub fn refresh(report: &Report, policy: &Policy, ctx: &ProviderContext) -> AppResult<Report> {
    let snapshot = providers::arch::sync_snapshot(ctx);
    let mut normalized = report.clone();
    if let Ok(snapshot) = &snapshot {
        crate::store::atomic_json(&ctx.paths.state.join("latest-snapshot.json"), snapshot)?;
        for app in &mut normalized.apps {
            for target in &mut app.components {
                if let Locator::Repo { package } = &target.locator
                    && snapshot.preferred(package).is_none()
                    && !report.checks.iter().any(|c| {
                        c.target_id == target.id
                            && c.source
                                .as_deref()
                                .is_some_and(|s| s.starts_with("Repositorio "))
                    })
                    && let Ok(base) = providers::aur::package_base(package, ctx)
                {
                    target.locator = Locator::Aur {
                        package: package.clone(),
                        base,
                    };
                }
            }
        }
    }
    let checked = refresh_with_provider(
        &normalized,
        policy,
        ctx,
        &Checks {
            arch: providers::arch::ArchProvider::new(snapshot),
        },
    )?;
    prune_snapshots(&ctx.paths)?;
    Ok(checked)
}
pub fn snapshot(paths: &Paths) -> AppResult<providers::arch::RepoSnapshot> {
    let bytes =
        crate::store::read_private(&paths.state.join("latest-snapshot.json"), 64 * 1024 * 1024)?
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::PlanChanged,
                    "Comprueba las versiones antes de preparar un plan",
                )
            })?;
    let snapshot: providers::arch::RepoSnapshot = serde_json::from_slice(&bytes)?;
    if crate::now() < snapshot.created_at
        || crate::now().saturating_sub(snapshot.created_at) > 21_600
    {
        return Err(AppError::new(
            ErrorCode::PlanExpired,
            "La comprobación de repositorios venció",
        ));
    }
    Ok(snapshot)
}
fn prune_snapshots(paths: &Paths) -> AppResult<()> {
    let root = paths.state.join("snapshots");
    if !root.exists() {
        return Ok(());
    }
    crate::store::private_dir(&root)?;
    let mut dirs = std::fs::read_dir(&root)?
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_ok_and(|t| t.is_dir())
                && uuid::Uuid::parse_str(&e.file_name().to_string_lossy()).is_ok()
        })
        .collect::<Vec<_>>();
    dirs.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));
    let count = dirs.len().saturating_sub(3);
    for dir in dirs.into_iter().take(count) {
        std::fs::remove_dir_all(dir.path())?;
    }
    Ok(())
}

pub fn plan_recipes(
    plan: &UpdatePlan,
    paths: &Paths,
) -> AppResult<std::collections::BTreeMap<TargetId, crate::execution::aur::AurRecipe>> {
    if !plan.steps.iter().any(|s| s.kind == StepKind::AurBuild) {
        return Ok(Default::default());
    }
    let bytes = crate::store::read_private(
        &paths
            .state
            .join("plans")
            .join(format!("{}.recipes.json", plan.id.0)),
        64 * 1024 * 1024,
    )?
    .ok_or_else(|| {
        AppError::new(
            ErrorCode::PlanChanged,
            "Faltan las recetas del plan revisado",
        )
    })?;
    let recipes: std::collections::BTreeMap<TargetId, crate::execution::aur::AurRecipe> =
        serde_json::from_slice(&bytes)?;
    for step in plan.steps.iter().filter(|s| s.kind == StepKind::AurBuild) {
        let reference = step
            .aur_recipe
            .as_ref()
            .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "Falta la referencia AUR"))?;
        if !recipes.values().any(|r| {
            r.digest == reference.digest
                && r.commit == reference.commit
                && r.package_base == reference.package_base
                && crate::execution::aur::recipe_digest(r) == r.digest
        }) {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "La receta AUR cambió",
            ));
        }
    }
    Ok(recipes)
}

pub fn prepare_plan(
    paths: &Paths,
    selection: &[AppId],
) -> AppResult<(UpdatePlan, Vec<crate::execution::aur::AurRecipe>)> {
    use crate::execution::aur::{AurRecipe, recipe_digest};
    use std::collections::{BTreeMap, BTreeSet};
    let store = crate::store::Store::new(paths.clone());
    let state = store.load()?;
    let policy = Policy::for_paths(paths)?;
    let snapshot = snapshot(paths)?;
    let ctx = ProviderContext::new(paths.clone());
    let mut recipes = BTreeMap::<TargetId, AurRecipe>::new();
    let mut by_base = BTreeMap::<String, AurRecipe>::new();
    let mut pending = state
        .report
        .apps
        .iter()
        .filter(|a| selection.contains(&a.id))
        .flat_map(|a| &a.components)
        .filter(|t| {
            matches!(t.locator, Locator::Aur { .. })
                && state.report.checks.iter().any(|c| {
                    c.target_id == t.id && !c.stale && c.state == CheckState::UpdateAvailable
                })
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    while let Some(target) = pending.pop() {
        let Locator::Aur { package, base } = &target.locator else {
            continue;
        };
        if policy.protected_package(package) {
            return Err(AppError::new(
                ErrorCode::CoverageNeedsReview,
                "Una receta afecta una herramienta de updater.py",
            ));
        }
        if !seen.insert(package.clone()) {
            continue;
        }
        if seen.len() > 64 {
            return Err(AppError::new(
                ErrorCode::Unsupported,
                "El árbol AUR excede el alcance revisable",
            ));
        }
        let mut recipe = if let Some(saved) = by_base.get(base) {
            saved.clone()
        } else {
            providers::aur::prepare_aur(&target, &ctx)?
        };
        if !recipe.packages.contains_key(package) {
            return Err(AppError::new(
                ErrorCode::PlanChanged,
                "El paquete ya no pertenece a la base AUR",
            ));
        }
        if !recipe.requested.contains(package) {
            recipe.requested.push(package.clone());
            recipe.digest = recipe_digest(&recipe);
            crate::store::atomic_json(&recipe.repository.join("review.json"), &recipe)?;
        }
        for dependency in &recipe.dependencies {
            if snapshot.installed.iter().any(|p| {
                crate::planner::satisfies(&dependency.expression, &p.name, &p.version, &p.provides)
            }) || snapshot.packages.iter().any(|p| {
                crate::planner::satisfies(&dependency.expression, &p.name, &p.version, &p.provides)
            }) {
                continue;
            }
            let name = dependency
                .expression
                .split(['<', '>', '='])
                .next()
                .unwrap_or("");
            let base = providers::aur::package_base(name, &ctx)?;
            pending.push(Target {
                id: format!("aur:{name}").into(),
                locator: Locator::Aur {
                    package: name.into(),
                    base,
                },
                installed: None,
            });
        }
        by_base.insert(base.clone(), recipe);
        recipes.insert(target.id.clone(), by_base[base].clone());
    }
    for recipe in recipes.values_mut() {
        *recipe = by_base[&recipe.package_base].clone();
    }
    let plan = crate::planner::build_plan_with_recipes(
        &state.report,
        selection,
        &snapshot,
        &policy,
        crate::now(),
        &recipes,
    )?;
    crate::store::atomic_json(
        &paths
            .state
            .join("plans")
            .join(format!("{}.recipes.json", plan.id.0)),
        &recipes,
    )?;
    store.update(|s| {
        crate::planner::validate_plan(&plan, &s.report, &policy, crate::now())?;
        s.plans.retain(|_, p| p.expires_at > crate::now());
        s.plans.insert(plan.id.clone(), plan.clone());
        Ok(())
    })?;
    Ok((plan, by_base.into_values().collect()))
}
