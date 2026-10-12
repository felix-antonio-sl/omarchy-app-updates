mod common;
use omarchy_app_updates::{
    domain::*,
    execution::{ExecutionContext, aur::*},
    helper::SilentSink,
    io::command::*,
    providers::{ProviderContext, aur::prepare_aur},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
struct Backend {
    repo: PathBuf,
    calls: Mutex<Vec<CommandSpec>>,
    fail: bool,
    unexpected: bool,
}
impl CommandBackend for Backend {
    fn run(&self, spec: &CommandSpec) -> AppResult<Captured> {
        self.calls.lock().unwrap().push(spec.clone());
        if spec.program == Path::new("/usr/bin/git") && spec.args.iter().any(|a| a == "clone") {
            let destination = spec.args.last().unwrap();
            let o = Command::new("/usr/bin/git")
                .args([
                    "clone",
                    "--no-checkout",
                    "--",
                    self.repo.to_str().unwrap(),
                    destination,
                ])
                .output()?;
            return Ok(Captured {
                code: o.status.code().unwrap_or(1),
                stdout: String::from_utf8_lossy(&o.stdout).into(),
                stderr: String::from_utf8_lossy(&o.stderr).into(),
            });
        }
        if spec.program == Path::new("/usr/bin/makepkg") {
            assert!(!spec.args.iter().any(|a| matches!(
                a.as_str(),
                "--syncdeps" | "--install" | "--nodeps" | "--skipinteg" | "--skippgpcheck"
            )));
            if self.fail {
                return Ok(Captured {
                    code: 4,
                    stdout: String::new(),
                    stderr: "fixture failed".into(),
                });
            }
            let dest = PathBuf::from(&spec.env["PKGDEST"]);
            for name in if self.unexpected {
                vec!["unreviewed"]
            } else {
                vec!["sample-app", "sample-docs"]
            } {
                archive(&dest, name, "2.0-1");
            }
            return Ok(Captured {
                code: 0,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        CommandRunner::default().run(spec)
    }
}
fn archive(dir: &Path, name: &str, version: &str) {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("usr/share/aur-fixture")).unwrap();
    fs::write(d.path().join(".PKGINFO"),format!("pkgname = {name}\npkgbase = sample\npkgver = {version}\npkgdesc = fixture\narch = x86_64\nsize = 7\nbuilddate = 1\npackager = fixture\n")).unwrap();
    fs::write(d.path().join("usr/share/aur-fixture/proof"), "fixture").unwrap();
    assert!(
        Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(dir.join(format!("{name}.pkg.tar.gz")))
            .arg("-C")
            .arg(d.path())
            .args([".PKGINFO", "usr"])
            .status()
            .unwrap()
            .success()
    );
}
fn fixture(home: &Path, fail: bool, unexpected: bool) -> (Target, ProviderContext, Arc<Backend>) {
    let (_, _, _, paths) = common::fixture(home);
    let repo = home.join("upstream");
    fs::create_dir(&repo).unwrap();
    assert!(
        Command::new("/usr/bin/git")
            .args(["init", "-q"])
            .arg(&repo)
            .status()
            .unwrap()
            .success()
    );
    fs::write(
        repo.join("PKGBUILD"),
        format!(
            "printf executed > '{}'\n# must never execute during review\n",
            home.join("executed").display()
        ),
    )
    .unwrap();
    fs::write(repo.join(".SRCINFO"),"pkgbase = sample\n\tpkgver = 2.0\n\tpkgrel = 1\n\tarch = x86_64\npkgname = sample-app\npkgname = sample-docs\n").unwrap();
    fs::write(repo.join("install.sh"), "#!/bin/sh\necho reviewed\n").unwrap();
    assert!(
        Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&repo)
            .args(["add", "."])
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.org",
                "commit",
                "-qm",
                "fixture"
            ])
            .status()
            .unwrap()
            .success()
    );
    let backend = Arc::new(Backend {
        repo,
        calls: Mutex::new(vec![]),
        fail,
        unexpected,
    });
    let mut ctx = ProviderContext::new(paths);
    ctx.commands = CommandRunner::with_backend(backend.clone());
    (
        Target {
            id: "aur:sample-app".into(),
            locator: Locator::Aur {
                package: "sample-app".into(),
                base: "sample".into(),
            },
            installed: Some(Version::arch("1.0-1")),
        },
        ctx,
        backend,
    )
}
#[test]
fn aur_review_is_data_only_and_pins_every_recipe_file() {
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, backend) = fixture(d.path(), false, false);
    let recipe = prepare_aur(&target, &ctx).unwrap();
    assert!(!d.path().join("executed").exists());
    assert!(recipe.files.iter().any(|f| f.path == "PKGBUILD"));
    assert!(recipe.files.iter().any(|f| f.path == "install.sh"));
    assert_eq!(recipe.packages.len(), 2);
    assert_eq!(recipe.requested, vec!["sample-app"]);
    assert_eq!(recipe.digest, recipe_digest(&recipe));
    assert!(
        !backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.program == Path::new("/usr/bin/makepkg"))
    );
}
#[test]
fn changed_commit_or_recipe_content_cannot_build() {
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, backend) = fixture(d.path(), false, false);
    let mut recipe = prepare_aur(&target, &ctx).unwrap();
    let exec = ExecutionContext::new(ctx, "op".into(), &SilentSink);
    recipe.commit = "a".repeat(40);
    assert!(build_aur(&recipe, &exec).is_err());
    assert!(
        !backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.program == Path::new("/usr/bin/makepkg"))
    );
}
#[test]
fn split_package_build_returns_only_reviewed_targets_and_rejects_unexpected_names() {
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, backend) = fixture(d.path(), false, false);
    let recipe = prepare_aur(&target, &ctx).unwrap();
    let exec = ExecutionContext::new(ctx, "op".into(), &SilentSink);
    let built = build_aur(&recipe, &exec).unwrap();
    assert_eq!(built.len(), 1);
    assert_eq!(built[0].name, "sample-app");
    assert_eq!(built[0].sha256.len(), 64);
    assert_eq!(
        backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.program == Path::new("/usr/bin/makepkg"))
            .count(),
        1
    );
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, _) = fixture(d.path(), false, true);
    let recipe = prepare_aur(&target, &ctx).unwrap();
    assert_eq!(
        build_aur(
            &recipe,
            &ExecutionContext::new(ctx, "op".into(), &SilentSink)
        )
        .unwrap_err()
        .code,
        ErrorCode::UnsafeArtifact
    );
}
#[test]
fn build_failure_is_not_retried_or_promoted_to_installation() {
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, backend) = fixture(d.path(), true, false);
    let recipe = prepare_aur(&target, &ctx).unwrap();
    assert_eq!(
        build_aur(
            &recipe,
            &ExecutionContext::new(ctx, "op".into(), &SilentSink)
        )
        .unwrap_err()
        .code,
        ErrorCode::TransactionFailed
    );
    assert_eq!(
        backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.program == Path::new("/usr/bin/makepkg"))
            .count(),
        1
    );
    assert!(
        !backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.program == Path::new("/usr/bin/pkexec")
                || s.program == Path::new("/usr/bin/pacman"))
    );
}
#[test]
fn root_and_unreviewed_transitive_dependencies_cannot_build() {
    assert_eq!(
        ensure_unprivileged(0).unwrap_err().code,
        ErrorCode::AuthUnavailable
    );
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, backend) = fixture(d.path(), false, false);
    let file = backend.repo.join(".SRCINFO");
    let contents = fs::read_to_string(&file).unwrap().replace(
        "\tarch = x86_64",
        "\tarch = x86_64\n\tmakedepends = missing-unreviewed-fixture-dep>=1",
    );
    fs::write(file, contents).unwrap();
    assert!(
        Command::new("/usr/bin/git")
            .arg("-C")
            .arg(&backend.repo)
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.org",
                "commit",
                "-qam",
                "dependency"
            ])
            .status()
            .unwrap()
            .success()
    );
    let recipe = prepare_aur(&target, &ctx).unwrap();
    assert!(
        build_aur(
            &recipe,
            &ExecutionContext::new(ctx, "op".into(), &SilentSink)
        )
        .is_err()
    );
    assert!(
        !backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|s| s.program == Path::new("/usr/bin/makepkg"))
    );
}

#[test]
fn aur_plan_includes_recipe_build_dependencies_and_requires_artifact_approval() {
    use omarchy_app_updates::planner::{build_plan_with_recipes, plan_digest};
    use std::collections::BTreeMap;
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, _) = fixture(d.path(), false, false);
    let recipe = prepare_aur(&target, &ctx).unwrap();
    let (mut report, mut snapshot, policy, _) = common::fixture(d.path());
    report.apps[0].id = target.id.0.clone().into();
    report.apps[0].components = vec![target.clone()];
    report.checks[0].target_id = target.id.clone();
    report.selected = vec![report.apps[0].id.clone()];
    snapshot.installed[0].name = "sample-app".into();
    snapshot.packages.clear();
    let recipes = BTreeMap::from([(target.id.clone(), recipe.clone())]);
    let plan =
        build_plan_with_recipes(&report, &report.selected, &snapshot, &policy, 100, &recipes)
            .unwrap();
    let build = plan
        .steps
        .iter()
        .find(|s| s.kind == StepKind::AurBuild)
        .unwrap();
    assert_eq!(build.aur_recipe.as_ref().unwrap().commit, recipe.commit);
    assert!(plan.steps.iter().any(|s| s.kind == StepKind::ArchFull));
    let artifacts = vec![BuiltArtifact {
        name: "sample-app".into(),
        version: "2.1-1".into(),
        path: d.path().join("package.pkg.tar.gz"),
        sha256: "a".repeat(64),
    }];
    let revised = attach_artifacts(&plan, &recipe, &artifacts, 200).unwrap();
    assert_ne!(revised.digest, plan.digest);
    assert_eq!(revised.digest, plan_digest(&revised));
    assert!(revised.notices.iter().any(|n| n.contains("2.1-1")));
    let mut changed = artifacts;
    changed[0].sha256 = "b".repeat(64);
    assert_ne!(
        attach_artifacts(&plan, &recipe, &changed, 200)
            .unwrap()
            .digest,
        revised.digest
    );
}
#[test]
fn selected_split_outputs_build_once_and_attach_in_one_reviewed_step() {
    use omarchy_app_updates::planner::build_plan_with_recipes;
    use std::collections::BTreeMap;
    let d = tempfile::tempdir().unwrap();
    let (first, ctx, backend) = fixture(d.path(), false, false);
    let mut recipe = prepare_aur(&first, &ctx).unwrap();
    recipe.requested.push("sample-docs".into());
    recipe.digest = recipe_digest(&recipe);
    fs::write(
        recipe.repository.join("review.json"),
        serde_json::to_vec(&recipe).unwrap(),
    )
    .unwrap();
    let (mut report, mut snapshot, policy, _) = common::fixture(d.path());
    report.apps.clear();
    report.checks.clear();
    report.selected.clear();
    snapshot.packages.clear();
    snapshot.installed.clear();
    let mut recipes = BTreeMap::new();
    for name in ["sample-app", "sample-docs"] {
        let target = Target {
            id: format!("aur:{name}").into(),
            locator: Locator::Aur {
                package: name.into(),
                base: "sample".into(),
            },
            installed: Some(Version::arch("1.0-1")),
        };
        let app = App {
            id: target.id.0.clone().into(),
            name: name.into(),
            desktop_ids: vec![],
            components: vec![target.clone()],
            suggested: true,
        };
        report.selected.push(app.id.clone());
        report.apps.push(app);
        report.checks.push(Check {
            target_id: target.id.clone(),
            installed: target.installed.clone(),
            available: Some(Version::arch("2.0-1")),
            published: None,
            checked_at: 100,
            state: CheckState::UpdateAvailable,
            stale: false,
            source: Some("AUR".into()),
            details: vec![],
        });
        recipes.insert(target.id, recipe.clone());
    }
    let plan =
        build_plan_with_recipes(&report, &report.selected, &snapshot, &policy, 100, &recipes)
            .unwrap();
    let steps = plan
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::AurBuild)
        .collect::<Vec<_>>();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].targets.len(), 2);
    assert_eq!(steps[0].package_changes.len(), 2);
    let artifacts = build_aur(
        &recipe,
        &ExecutionContext::new(ctx, "fixture".into(), &SilentSink),
    )
    .unwrap();
    assert_eq!(artifacts.len(), 2);
    let revised = attach_artifacts(&plan, &recipe, &artifacts, 200).unwrap();
    assert_eq!(
        revised
            .steps
            .iter()
            .find(|s| s.kind == StepKind::AurBuild)
            .unwrap()
            .built_artifacts
            .len(),
        2
    );
    assert_eq!(
        backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.program == Path::new("/usr/bin/makepkg"))
            .count(),
        1
    );
}
#[test]
fn dependency_recipes_are_resolved_transitively_and_cycles_need_review() {
    use std::collections::BTreeMap;
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, _) = fixture(d.path(), false, false);
    let mut main = prepare_aur(&target, &ctx).unwrap();
    main.dependencies.push(AurDependency {
        expression: "sample-helper>=2".into(),
        kind: "make".into(),
    });
    main.digest = recipe_digest(&main);
    let mut helper = main.clone();
    helper.package_base = "sample-helper".into();
    helper.dependencies.clear();
    helper.requested = vec!["sample-helper".into()];
    helper.packages = BTreeMap::from([("sample-helper".into(), "2.0-1".into())]);
    helper.digest = recipe_digest(&helper);
    let (_, snapshot, policy, _) = common::fixture(d.path());
    assert!(resolve_dependencies(&main, &snapshot, &BTreeMap::new(), &policy).is_err());
    let recipes = BTreeMap::from([("sample-helper".into(), helper.clone())]);
    assert_eq!(
        resolve_dependencies(&main, &snapshot, &recipes, &policy)
            .unwrap()
            .aur_bases,
        vec!["sample-helper"]
    );
    helper.dependencies = main.dependencies.clone();
    helper.digest = recipe_digest(&helper);
    assert!(
        resolve_dependencies(
            &main,
            &snapshot,
            &BTreeMap::from([("sample-helper".into(), helper)]),
            &policy
        )
        .is_err()
    );
}
#[test]
fn administrative_artifact_paths_are_limited_to_the_callers_build_outputs() {
    use omarchy_app_updates::helper::{staging::validate_build_artifact_path, validate::Caller};
    let caller = Caller {
        uid: 1000,
        gid: 1000,
        home: "/home/operator".into(),
    };
    assert!(validate_build_artifact_path(&PathBuf::from("/home/operator/.local/state/omarchy-app-updates/aur/builds/00000000-0000-4000-8000-000000000000/artifacts/app.pkg.tar.zst"),&caller).is_ok());
    for path in [
        "/tmp/app.pkg.tar.zst",
        "/home/operator/.ssh/id_rsa",
        "/home/other/.local/state/omarchy-app-updates/aur/builds/00000000-0000-4000-8000-000000000000/artifacts/app.pkg.tar.zst",
    ] {
        assert!(validate_build_artifact_path(Path::new(path), &caller).is_err());
    }
}

#[test]
fn system_scope_receipts_survive_a_later_aur_build_failure() {
    use omarchy_app_updates::execution::aur::preserve_build_failure;
    let d = tempfile::tempdir().unwrap();
    let (target, ctx, _) = fixture(d.path(), true, false);
    let recipe = prepare_aur(&target, &ctx).unwrap();
    let system = vec![TargetResult {
        target_id: "repo:system-library".into(),
        outcome: Outcome::Updated,
        old_version: Some(Version::arch("1.0-1")),
        new_version: Some(Version::arch("2.0-1")),
        error: None,
    }];
    let error = build_aur(
        &recipe,
        &ExecutionContext::new(ctx, "op".into(), &SilentSink),
    )
    .unwrap_err();
    let results = preserve_build_failure(system, &[target], &error);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].outcome, Outcome::Updated);
    assert_eq!(results[0].new_version.as_ref().unwrap().raw, "2.0-1");
    assert_eq!(results[1].outcome, Outcome::Failed);
    assert!(results[1].new_version.is_none());
}

#[test]
fn recipe_directory_creation_cannot_write_through_a_parent_symlink() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let elsewhere = d.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    symlink(&elsewhere, d.path().join("redirect")).unwrap();
    assert!(private_directory(&d.path().join("redirect/created")).is_err());
    assert!(!elsewhere.join("created").exists());
}
