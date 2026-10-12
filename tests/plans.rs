use omarchy_app_updates::{
    alpm::{AvailablePackage, PackageFacts, RepoConfig},
    domain::*,
    planner::{build_plan, plan_digest, validate_plan},
    policy::Policy,
    providers::arch::RepoSnapshot,
};
use std::path::Path;
fn fact(name: &str) -> PackageFacts {
    PackageFacts {
        name: name.into(),
        version: "1.0-1".into(),
        explicit: true,
        files: vec![format!("usr/bin/{name}")],
        url: String::new(),
        depends: vec![],
        provides: vec![],
        groups: vec![],
        ignored: false,
    }
}
fn package(name: &str) -> AvailablePackage {
    AvailablePackage {
        name: name.into(),
        version: "2.0-1".into(),
        repository: "extra".into(),
        download_bytes: 100,
        filename: format!("{name}-2.0-1.pkg.tar.zst"),
        sha256: "a".repeat(64),
        depends: vec![],
        provides: vec![],
        files: vec![],
    }
}
fn setup(home: &Path) -> (Report, RepoSnapshot, Policy) {
    let source = home.join(".local/share/ai-updaters/updater.py");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source,"TARGETS = ('codex', 'claude', 'antigravity', 'opencode', 'hermes', 'claude-desktop', 'chatgpt')\n").unwrap();
    let policy = Policy::load(&source, &home.join("catalog.json")).unwrap();
    let target = Target {
        id: "repo:ghostty".into(),
        locator: Locator::Repo {
            package: "ghostty".into(),
        },
        installed: Some(Version::arch("1.0-1")),
    };
    let report = Report {
        apps: vec![App {
            id: "repo:ghostty".into(),
            name: "Ghostty".into(),
            desktop_ids: vec!["ghostty.desktop".into()],
            components: vec![target.clone()],
            suggested: true,
        }],
        selected: vec!["repo:ghostty".into()],
        coverage: Coverage::Verified,
        checks: vec![Check {
            target_id: target.id,
            checked_at: 100,
            installed: target.installed,
            available: Some(Version::arch("2.0-1")),
            published: None,
            state: CheckState::UpdateAvailable,
            stale: false,
            source: Some("Repositorio extra".into()),
            details: vec![],
        }],
        operation: None,
        checked_at: Some(100),
    };
    let mut ghostty = package("ghostty");
    ghostty.depends = vec!["new-lib>=2.0".into()];
    let snapshot = RepoSnapshot {
        repositories: vec![RepoConfig {
            name: "extra".into(),
            servers: vec!["https://example.org".into()],
            siglevel: vec!["Required".into()],
            usage: vec![],
        }],
        packages: vec![
            ghostty,
            package("libc"),
            package("thunderbird"),
            package("thunderbird-i18n-es-es"),
            package("new-lib"),
        ],
        installed: vec![
            fact("ghostty"),
            fact("libc"),
            fact("thunderbird"),
            fact("thunderbird-i18n-es-es"),
        ],
        config_digest: "root-config".into(),
        fingerprint: "fixture".into(),
        dbpath: home.into(),
        created_at: 100,
    };
    (report, snapshot, policy)
}
#[test]
fn reviewed_scope_contains_system_languages_and_new_dependencies() {
    let d = tempfile::tempdir().unwrap();
    let (r, s, p) = setup(d.path());
    let plan = build_plan(&r, &r.selected, &s, &p, 100).unwrap();
    assert_eq!(plan.steps.len(), 1);
    let step = &plan.steps[0];
    assert_eq!(step.kind, StepKind::ArchFull);
    let names = step
        .package_changes
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "ghostty",
            "libc",
            "new-lib",
            "thunderbird",
            "thunderbird-i18n-es-es"
        ]
    );
    assert!(
        step.package_changes
            .iter()
            .any(|c| c.name == "libc" && c.reason == "system")
    );
    assert!(
        step.package_changes
            .iter()
            .any(|c| c.name == "new-lib" && c.old_version.is_none() && c.reason == "dependency")
    );
    assert_eq!(plan.expires_at, 1000);
    assert_eq!(plan.digest, plan_digest(&plan));
    assert!(!plan.notices.is_empty());
    validate_plan(&plan, &r, &p, 999).unwrap();
    assert_eq!(
        validate_plan(&plan, &r, &p, 1000).unwrap_err().code,
        ErrorCode::PlanExpired
    );
    assert!(validate_plan(&plan, &r, &p, 99).is_err());
}
#[test]
fn selected_flatpaks_share_one_prepared_transaction_and_runtime() {
    use omarchy_app_updates::{
        execution::flatpak::FlatpakBackend, planner::build_plan_with_flatpak,
    };
    struct Flatpaks(std::sync::atomic::AtomicUsize);
    impl FlatpakBackend for Flatpaks {
        fn prepare(&self, _: &Target, _: &str) -> AppResult<Vec<FlatpakChange>> {
            panic!("must prepare selected apps together")
        }
        fn prepare_many(&self, targets: &[(&Target, &str)]) -> AppResult<Vec<FlatpakChange>> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(targets.len(), 2);
            let mut changes = targets
                .iter()
                .map(|(t, c)| FlatpakChange {
                    installation: FlatpakInstall::User,
                    reference: omarchy_app_updates::providers::flatpak::target_ref(t)
                        .unwrap()
                        .1,
                    remote: "fixture".into(),
                    commit: (*c).into(),
                    previous_commit: Some("a".repeat(64)),
                    operation_type: 1,
                    download_bytes: 1,
                    metadata: String::new(),
                    old_metadata: String::new(),
                })
                .collect::<Vec<_>>();
            let mut runtime = changes[0].clone();
            runtime.reference = "runtime/org.test.Runtime/x86_64/stable".into();
            changes.push(runtime);
            Ok(changes)
        }
        fn apply(&self, _: &[FlatpakChange]) -> AppResult<()> {
            panic!("planning must never deploy")
        }
    }
    let d = tempfile::tempdir().unwrap();
    let (mut report, snapshot, policy) = setup(d.path());
    report.apps.clear();
    report.checks.clear();
    report.selected.clear();
    for name in ["org.test.One", "org.test.Two"] {
        let id = format!("flatpak:user:{name}/x86_64/stable");
        let target = Target {
            id: id.clone().into(),
            locator: Locator::Flatpak {
                installation: FlatpakInstall::User,
                app_id: name.into(),
                arch: "x86_64".into(),
                branch: "stable".into(),
                origin: "fixture".into(),
            },
            installed: Some(Version {
                raw: "a".repeat(64),
                kind: VersionKind::FlatpakCommit,
                product: None,
            }),
        };
        report.selected.push(id.clone().into());
        report.apps.push(App {
            id: id.into(),
            name: name.into(),
            desktop_ids: vec![],
            components: vec![target.clone()],
            suggested: true,
        });
        report.checks.push(Check {
            target_id: target.id,
            installed: target.installed,
            available: Some(Version {
                raw: "b".repeat(64),
                kind: VersionKind::FlatpakCommit,
                product: None,
            }),
            published: None,
            checked_at: 100,
            state: CheckState::UpdateAvailable,
            stale: false,
            source: None,
            details: vec![],
        });
    }
    let flatpaks = Flatpaks(std::sync::atomic::AtomicUsize::new(0));
    let plan = build_plan_with_flatpak(
        &report,
        &report.selected,
        &snapshot,
        &policy,
        100,
        &Default::default(),
        &flatpaks,
    )
    .unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].targets.len(), 2);
    assert_eq!(plan.steps[0].flatpak_changes.len(), 3);
    assert_eq!(flatpaks.0.load(std::sync::atomic::Ordering::Relaxed), 1);
}
#[test]
fn versions_dependencies_policy_and_source_changes_invalidate_review() {
    let d = tempfile::tempdir().unwrap();
    let (r, mut s, p) = setup(d.path());
    let plan = build_plan(&r, &r.selected, &s, &p, 100).unwrap();
    s.packages[0].version = "2.1-1".into();
    let changed = build_plan(&r, &r.selected, &s, &p, 100).unwrap();
    assert_ne!(plan.digest, changed.digest);
    s.packages[0].depends.push("extra-lib>=3".into());
    let mut lib = package("extra-lib");
    lib.version = "3.0-1".into();
    s.packages.push(lib);
    assert_ne!(
        changed.digest,
        build_plan(&r, &r.selected, &s, &p, 100).unwrap().digest
    );
    let mut r2 = r.clone();
    r2.apps[0].components[0].installed = Some(Version::arch("2.0-1"));
    assert_eq!(
        validate_plan(&plan, &r2, &p, 101).unwrap_err().code,
        ErrorCode::PlanChanged
    );
    std::fs::write(d.path().join(".local/share/ai-updaters/updater.py"),"TARGETS = ('codex', 'claude', 'antigravity', 'opencode', 'hermes', 'claude-desktop', 'chatgpt')\n# edited\n").unwrap();
    let p2 = Policy::load(
        &d.path().join(".local/share/ai-updaters/updater.py"),
        &d.path().join("missing"),
    )
    .unwrap();
    assert_eq!(
        validate_plan(&plan, &r, &p2, 101).unwrap_err().code,
        ErrorCode::PlanChanged
    );
}
#[test]
fn ignored_or_protected_full_scope_blocks_the_entire_plan() {
    let d = tempfile::tempdir().unwrap();
    let (r, mut s, p) = setup(d.path());
    s.installed[1].ignored = true;
    assert_eq!(
        build_plan(&r, &r.selected, &s, &p, 100).unwrap_err().code,
        ErrorCode::PlanChanged
    );
    s.installed[1].ignored = false;
    s.installed.push(fact("opencode"));
    s.packages.push(package("opencode"));
    assert_eq!(
        build_plan(&r, &r.selected, &s, &p, 100).unwrap_err().code,
        ErrorCode::CoverageNeedsReview
    );
    s.installed.pop();
    s.packages.pop();
    s.packages[1].files = vec!["usr/bin/opencode".into()];
    assert_eq!(
        build_plan(&r, &r.selected, &s, &p, 100).unwrap_err().code,
        ErrorCode::CoverageNeedsReview
    );
}
#[test]
fn unknown_web_and_incomplete_coverage_do_not_install() {
    let d = tempfile::tempdir().unwrap();
    let (mut r, s, p) = setup(d.path());
    r.apps[0].components[0].locator = Locator::Web {
        url: "https://web.whatsapp.com".into(),
        browser: None,
    };
    assert!(
        build_plan(&r, &r.selected, &s, &p, 100)
            .unwrap()
            .steps
            .is_empty()
    );
    r.apps[0].components[0].locator = Locator::Unknown { path: None };
    assert_eq!(
        build_plan(&r, &r.selected, &s, &p, 100).unwrap_err().code,
        ErrorCode::Unsupported
    );
    r.coverage = Coverage::NeedsReview {
        reason: "changed".into(),
        unknown: vec![],
    };
    assert_eq!(
        build_plan(&r, &r.selected, &s, &p, 100).unwrap_err().code,
        ErrorCode::CoverageNeedsReview
    );
}
#[test]
fn unavailable_dependency_or_multiple_virtual_providers_requires_review() {
    let d = tempfile::tempdir().unwrap();
    let (r, mut s, p) = setup(d.path());
    s.packages[0].depends = vec!["missing>=2".into()];
    assert!(build_plan(&r, &r.selected, &s, &p, 100).is_err());
    s.packages[0].depends = vec!["virtual-lib>=2".into()];
    for name in ["impl-one", "impl-two"] {
        let mut v = package(name);
        v.provides = vec!["virtual-lib=2.0".into()];
        s.packages.push(v);
    }
    assert!(build_plan(&r, &r.selected, &s, &p, 100).is_err());
}
#[test]
fn canonical_digest_covers_every_visible_step_without_order_noise() {
    let d = tempfile::tempdir().unwrap();
    let (r, s, p) = setup(d.path());
    let a = build_plan(&r, &r.selected, &s, &p, 100).unwrap();
    let mut b = a.clone();
    b.steps[0].package_changes.reverse();
    assert_eq!(plan_digest(&a), plan_digest(&b));
    b.steps[0].package_changes[0].new_version = "999-1".into();
    assert_ne!(plan_digest(&a), plan_digest(&b));
    b = a.clone();
    b.steps[0].downloads.push(Download {
        url: "https://example.org/other".into(),
        expected_sha256: None,
        bytes: None,
    });
    assert_ne!(plan_digest(&a), plan_digest(&b));
}

#[test]
fn unrepresentable_launcher_path_returns_an_error_without_crashing() {
    use std::os::unix::ffi::OsStringExt;
    let d = tempfile::tempdir().unwrap();
    let (mut r, s, p) = setup(d.path());
    r.apps.push(App {
        id: "unknown-extra".into(),
        name: "Other".into(),
        desktop_ids: vec![],
        components: vec![Target {
            id: "unknown-extra".into(),
            locator: Locator::Unknown {
                path: Some(std::ffi::OsString::from_vec(vec![b'/', 0xff]).into()),
            },
            installed: None,
        }],
        suggested: false,
    });
    assert_eq!(
        build_plan(&r, &r.selected, &s, &p, 100).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}
#[test]
fn flatpak_dependency_commits_and_permissions_are_part_of_review_digest() {
    let d = tempfile::tempdir().unwrap();
    let (r, s, p) = setup(d.path());
    let mut plan = build_plan(&r, &r.selected, &s, &p, 100).unwrap();
    plan.steps.push(PlanStep {
        built_artifacts: vec![],
        build_dependencies: vec![],
        kind: StepKind::FlatpakUpdate,
        targets: vec!["flatpak:app".into()],
        package_changes: vec![],
        downloads: vec![],
        requires_auth: false,
        reopen: vec![],
        aur_recipe: None,
        flatpak_changes: vec![FlatpakChange {
            installation: FlatpakInstall::User,
            reference: "runtime/org.example.Runtime/x86_64/stable".into(),
            remote: "flathub".into(),
            commit: "b".repeat(64),
            previous_commit: Some("a".repeat(64)),
            operation_type: 1,
            download_bytes: 100,
            metadata: "[Context]\nshared=network;\n".into(),
            old_metadata: String::new(),
        }],
    });
    let before = plan_digest(&plan);
    plan.steps[1].flatpak_changes[0].commit = "c".repeat(64);
    assert_ne!(before, plan_digest(&plan));
    let before = plan_digest(&plan);
    plan.steps[1].flatpak_changes[0]
        .metadata
        .push_str("filesystems=host;\n");
    assert_ne!(before, plan_digest(&plan));
}
