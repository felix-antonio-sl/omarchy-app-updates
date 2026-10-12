use omarchy_app_updates::{
    domain::*,
    execution::{
        EventSink, ExecutionContext,
        flatpak::{FlatpakBackend, run_flatpak},
    },
    io::{command::*, http::HttpClient},
    paths::Paths,
    providers::{
        Provider, ProviderContext,
        flatpak::{FlatpakProvider, flatpak_scope},
    },
    store::Store,
};
use std::sync::{Arc, Mutex};
struct FlatpakCommands {
    calls: Arc<Mutex<Vec<Vec<String>>>>,
    remote: String,
}
impl CommandBackend for FlatpakCommands {
    fn run(&self, s: &CommandSpec) -> AppResult<Captured> {
        self.calls.lock().unwrap().push(s.args.clone());
        let output = if s.args.iter().any(|a| a == "--show-commit") {
            if s.args[0] == "remote-info" {
                self.remote.clone()
            } else {
                "a".repeat(64)
            }
        } else if s.args.iter().any(|a| a == "--show-origin") {
            "flathub".into()
        } else if s.args.iter().any(|a| a == "--show-runtime") {
            "org.freedesktop.Platform/x86_64/24.08".into()
        } else {
            "End-of-life: unsupported runtime".into()
        };
        Ok(Captured {
            code: 0,
            stdout: output,
            stderr: String::new(),
        })
    }
}
fn target(scope: FlatpakInstall, branch: &str) -> Target {
    Target {
        id: format!("flatpak:{scope:?}:org.example.App/x86_64/{branch}").into(),
        locator: Locator::Flatpak {
            installation: scope,
            app_id: "org.example.App".into(),
            arch: "x86_64".into(),
            branch: branch.into(),
            origin: String::new(),
        },
        installed: None,
    }
}
#[test]
fn flatpak_installations_branches_and_commits_are_independent() {
    let d = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(vec![]));
    let commands = CommandRunner::with_backend(Arc::new(FlatpakCommands {
        calls: calls.clone(),
        remote: "b".repeat(64),
    }));
    let ctx = ProviderContext {
        paths: Paths::for_home(d.path().into()),
        commands,
        http: HttpClient::default(),
        now: 100,
    };
    let user = target(FlatpakInstall::User, "stable");
    let system = target(
        FlatpakInstall::System {
            name: "corp".into(),
        },
        "beta",
    );
    let c = FlatpakProvider.check(&user, &ctx).unwrap();
    assert_eq!(c.state, CheckState::UpdateAvailable);
    assert_eq!(c.available.unwrap().raw, "b".repeat(64));
    assert!(c.details.iter().any(|d| d.contains("24.08")));
    FlatpakProvider.check(&system, &ctx).unwrap();
    let calls = calls.lock().unwrap();
    assert!(
        calls
            .iter()
            .any(|a| a.contains(&"--user".into()) && a.iter().any(|s| s.ends_with("/stable")))
    );
    assert!(calls.iter().any(
        |a| a.contains(&"--installation=corp".into()) && a.iter().any(|s| s.ends_with("/beta"))
    ));
    assert_ne!(user.id, system.id);
    assert!(
        flatpak_scope(&FlatpakInstall::System {
            name: "--evil".into()
        })
        .is_err()
    );
}
#[test]
fn matching_commit_confirms_current_even_without_version_metadata() {
    let d = tempfile::tempdir().unwrap();
    let ctx = ProviderContext {
        paths: Paths::for_home(d.path().into()),
        commands: CommandRunner::with_backend(Arc::new(FlatpakCommands {
            calls: Arc::new(Mutex::new(vec![])),
            remote: "a".repeat(64),
        })),
        http: HttpClient::default(),
        now: 100,
    };
    assert_eq!(
        FlatpakProvider
            .check(&target(FlatpakInstall::User, "stable"), &ctx)
            .unwrap()
            .state,
        CheckState::Current
    );
}
struct NoopBackend;
impl FlatpakBackend for NoopBackend {
    fn prepare(&self, _: &Target, _: &str) -> AppResult<Vec<FlatpakChange>> {
        Ok(vec![])
    }
    fn apply(&self, _: &[FlatpakChange]) -> AppResult<()> {
        Ok(())
    }
}
struct Sink;
impl EventSink for Sink {
    fn send(&self, _: OperationEvent) -> AppResult<()> {
        Ok(())
    }
}
#[test]
fn deployment_requires_verified_commit_not_process_success() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().into());
    let t = target(FlatpakInstall::User, "stable");
    let store = Store::new(paths.clone());
    let mut state = store.load().unwrap();
    state.report.apps.push(App {
        id: "flatpak:user:app".into(),
        name: "App".into(),
        desktop_ids: vec![],
        components: vec![t.clone()],
        suggested: true,
    });
    store.save(&state).unwrap();
    let provider = ProviderContext {
        paths: paths.clone(),
        commands: CommandRunner::with_backend(Arc::new(FlatpakCommands {
            calls: Arc::new(Mutex::new(vec![])),
            remote: "b".repeat(64),
        })),
        http: HttpClient::default(),
        now: 100,
    };
    let ctx = ExecutionContext::new(provider, "test-operation".into(), &Sink)
        .with_flatpak(Arc::new(NoopBackend));
    let step = PlanStep {
        kind: StepKind::FlatpakUpdate,
        targets: vec![t.id],
        package_changes: vec![],
        downloads: vec![],
        requires_auth: false,
        reopen: vec![],
        aur_recipe: None,
        flatpak_changes: vec![FlatpakChange {
            installation: FlatpakInstall::User,
            reference: "app/org.example.App/x86_64/stable".into(),
            remote: "flathub".into(),
            commit: "b".repeat(64),
            previous_commit: Some("a".repeat(64)),
            operation_type: 1,
            download_bytes: 1,
            metadata: String::new(),
            old_metadata: String::new(),
        }],
    };
    let results = run_flatpak(&step, &ctx).unwrap();
    assert_eq!(results[0].outcome, Outcome::Failed);
}

struct CountingBackend(Arc<std::sync::atomic::AtomicUsize>);
impl FlatpakBackend for CountingBackend {
    fn prepare(&self, _: &Target, _: &str) -> AppResult<Vec<FlatpakChange>> {
        Ok(vec![])
    }
    fn apply(&self, _: &[FlatpakChange]) -> AppResult<()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
fn change(reference: &str) -> FlatpakChange {
    FlatpakChange {
        installation: FlatpakInstall::User,
        reference: reference.into(),
        remote: "flathub".into(),
        commit: "b".repeat(64),
        previous_commit: Some("a".repeat(64)),
        operation_type: 1,
        download_bytes: 1,
        metadata: String::new(),
        old_metadata: String::new(),
    }
}
fn step(ids: Vec<TargetId>, changes: Vec<FlatpakChange>) -> PlanStep {
    PlanStep {
        kind: StepKind::FlatpakUpdate,
        targets: ids,
        flatpak_changes: changes,
        package_changes: vec![],
        downloads: vec![],
        requires_auth: false,
        reopen: vec![],
        aur_recipe: None,
    }
}
#[test]
fn removed_target_is_rejected_before_deployment() {
    let d = tempfile::tempdir().unwrap();
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ctx = ExecutionContext::new(
        ProviderContext::new(Paths::for_home(d.path().into())),
        "op".into(),
        &Sink,
    )
    .with_flatpak(Arc::new(CountingBackend(count.clone())));
    assert_eq!(
        run_flatpak(
            &step(
                vec!["missing".into()],
                vec![change("app/org.example.App/x86_64/stable")]
            ),
            &ctx
        )
        .unwrap_err()
        .code,
        ErrorCode::PlanChanged
    );
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
}
struct AppOnlyUpdated;
impl CommandBackend for AppOnlyUpdated {
    fn run(&self, s: &CommandSpec) -> AppResult<Captured> {
        Ok(Captured {
            code: 0,
            stdout: if s.args.last().unwrap().starts_with("app/") {
                "b".repeat(64)
            } else {
                "a".repeat(64)
            },
            stderr: String::new(),
        })
    }
}
#[test]
fn successful_app_commit_does_not_hide_failed_runtime_deployment() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().into());
    let store = Store::new(paths.clone());
    let t = target(FlatpakInstall::User, "stable");
    let mut state = store.load().unwrap();
    state.report.apps.push(App {
        id: "app".into(),
        name: "App".into(),
        desktop_ids: vec![],
        components: vec![t.clone()],
        suggested: true,
    });
    store.save(&state).unwrap();
    let provider = ProviderContext {
        paths,
        commands: CommandRunner::with_backend(Arc::new(AppOnlyUpdated)),
        http: HttpClient::default(),
        now: 100,
    };
    let ctx =
        ExecutionContext::new(provider, "op".into(), &Sink).with_flatpak(Arc::new(NoopBackend));
    let result = run_flatpak(
        &step(
            vec![t.id],
            vec![
                change("app/org.example.App/x86_64/stable"),
                change("runtime/org.example.Runtime/x86_64/stable"),
            ],
        ),
        &ctx,
    )
    .unwrap();
    assert_eq!(result[0].outcome, Outcome::Failed);
}
#[test]
#[ignore = "host read-only network acceptance"]
fn host_geforce_now_check_and_prepare_do_not_deploy() {
    use omarchy_app_updates::execution::flatpak::prepare_flatpak;
    let ctx = ProviderContext::new(Paths::for_home("/home/korvo".into()));
    let t = Target {
        id: "host-gfn".into(),
        locator: Locator::Flatpak {
            installation: FlatpakInstall::User,
            app_id: "com.nvidia.geforcenow".into(),
            arch: "x86_64".into(),
            branch: "master".into(),
            origin: "GeForceNOW".into(),
        },
        installed: None,
    };
    let before = ctx
        .commands
        .run(
            &CommandSpec::new("/usr/bin/flatpak").args(
                [
                    "info",
                    "--user",
                    "--show-commit",
                    "app/com.nvidia.geforcenow/x86_64/master",
                ]
                .map(String::from),
            ),
        )
        .unwrap()
        .stdout;
    let c = FlatpakProvider.check(&t, &ctx).unwrap();
    assert!(matches!(
        c.state,
        CheckState::Current | CheckState::UpdateAvailable
    ));
    let _operations = prepare_flatpak(&t, &c.available.unwrap().raw).unwrap();
    let after = ctx
        .commands
        .run(
            &CommandSpec::new("/usr/bin/flatpak").args(
                [
                    "info",
                    "--user",
                    "--show-commit",
                    "app/com.nvidia.geforcenow/x86_64/master",
                ]
                .map(String::from),
            ),
        )
        .unwrap()
        .stdout;
    assert_eq!(before, after, "preparation must not deploy");
}
