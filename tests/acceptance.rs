mod common;
use omarchy_app_updates::{
    domain::*,
    execution::{ExecutionContext, reopen::*, telegram::*},
    helper::DecisionValue,
    inventory,
    io::{command::*, http::*},
    jobs::{self, control::*, launch::*, refresh::refresh_with_provider, worker::JobHandle},
    planner::{build_plan, plan_digest, validate_plan},
    policy::Policy,
    protocol::{Action, Request, dispatch},
    providers::{Provider, ProviderContext, check_base, telegram::*},
    store::Store,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
};
struct Commands;
impl CommandBackend for Commands {
    fn run(&self, _: &CommandSpec) -> AppResult<Captured> {
        Ok(Captured {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}
struct Http(Vec<u8>);
impl HttpTransport for Http {
    fn send(&self, url: &str, _: &DownloadPolicy) -> AppResult<HttpResponse> {
        Ok(HttpResponse {
            url: url.into(),
            status: 200,
            bytes: self.0.clone(),
            headers: BTreeMap::new(),
        })
    }
}
#[derive(Default)]
struct Checks(Mutex<Vec<TargetId>>);
impl Provider for Checks {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        self.0.lock().unwrap().push(target.id.clone());
        let mut check = check_base(target, ctx, CheckState::Unsupported);
        if matches!(target.locator, Locator::TelegramLocal { .. }) {
            check.installed = Some(Version::product("1.0"));
            check.available = Some(Version::product("2.0"));
            check.published = check.available.clone();
            check.state = CheckState::UpdateAvailable;
            check.source = Some("https://telegram.org/dl/desktop/linux".into());
        }
        Ok(check)
    }
}
#[derive(Default)]
struct Jobs(Mutex<Vec<LaunchSpec>>);
impl JobBackend for Jobs {
    fn launch(&self, spec: &LaunchSpec) -> AppResult<()> {
        self.0.lock().unwrap().push(spec.clone());
        Ok(())
    }
    fn state(&self, _: &str) -> AppResult<UnitState> {
        Ok(UnitState::Active)
    }
}
struct Processes;
impl ProcessBackend for Processes {
    fn running(&self, _: &[PathBuf]) -> AppResult<Vec<RunningApp>> {
        Ok(vec![])
    }
    fn matches(&self, _: &RunningApp) -> AppResult<bool> {
        Ok(false)
    }
    fn close(&self, _: &RunningApp) -> AppResult<()> {
        panic!("acceptance must never close a process")
    }
    fn launch(&self, _: &App, _: &omarchy_app_updates::paths::Paths) -> AppResult<()> {
        panic!("acceptance must never launch a desktop app")
    }
}
fn elf(marker: u8) -> Vec<u8> {
    let mut b = vec![0; 64];
    b[..4].copy_from_slice(b"\x7fELF");
    b[4] = 2;
    b[5] = 1;
    b[6] = 1;
    b[16] = 2;
    b[18] = 62;
    b[48] = marker;
    b
}
#[test]
fn whole_flow_in_isolated_environment() {
    let d = tempfile::tempdir().unwrap();
    let (_, snapshot, _, mut paths) = common::fixture(d.path());
    paths.runtime_dir = d.path().join("run");
    let desktops = d.path().join("applications");
    fs::create_dir(&desktops).unwrap();
    paths.application_dirs = vec![desktops.clone()];
    let root = d.path().join(".local/opt/telegram-desktop");
    fs::create_dir_all(&root).unwrap();
    for (name, mark) in [("Telegram", 1), ("Updater", 0)] {
        fs::write(root.join(name), elf(mark)).unwrap();
        fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        root.join(".app-updates-version.json"),
        serde_json::to_vec(&TelegramReceipt {
            version: "1.0".into(),
            binary_sha256: binary_hash(&root.join("Telegram")).unwrap(),
        })
        .unwrap(),
    )
    .unwrap();
    let profile = d.path().join(".local/share/TelegramDesktop/tdata");
    fs::create_dir_all(&profile).unwrap();
    fs::write(profile.join("account"), b"preserve fixture account").unwrap();
    fs::write(
        desktops.join("telegram.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Telegram Desktop\nExec={}\n",
            root.join("Telegram").display()
        ),
    )
    .unwrap();
    fs::write(
        desktops.join("codex.desktop"),
        "[Desktop Entry]\nType=Application\nName=Codex\nExec=codex\n",
    )
    .unwrap();
    fs::write(
        desktops.join("unknown.desktop"),
        "[Desktop Entry]\nType=Application\nName=Unsupported app\nExec=/nonexistent/fixture-app\n",
    )
    .unwrap();
    let commands = CommandRunner::with_backend(Arc::new(Commands));
    let apps = inventory::discover(&paths, &[], &commands).unwrap();
    assert_eq!(apps.len(), 3);
    let policy = Policy::for_paths(&paths).unwrap();
    let mut report = Report {
        apps,
        ..Default::default()
    };
    policy
        .filter_report(&mut report, &UserConfig::default())
        .unwrap();
    assert_eq!(report.apps.len(), 2);
    assert!(!report.apps.iter().any(|a| a.name == "Codex"));
    let app = report
        .apps
        .iter()
        .find(|a| a.name == "Telegram Desktop")
        .unwrap()
        .clone();
    let unsupported = report
        .apps
        .iter()
        .find(|a| a.name == "Unsupported app")
        .unwrap()
        .clone();
    let store = Store::new(paths.clone());
    store
        .save(&State {
            report,
            ..Default::default()
        })
        .unwrap();
    let selected = dispatch(
        Request {
            protocol_version: 1,
            request_id: "select-fixture".into(),
            action: Action::Selection,
            payload: serde_json::json!({"selected":[app.id,unsupported.id]}),
        },
        &paths,
    )
    .unwrap();
    assert!(selected.error.is_none());
    let mut ctx = ProviderContext::new(paths.clone());
    ctx.commands = commands;
    let now = ctx.now;
    let checks = Checks::default();
    let mut report =
        refresh_with_provider(&store.load().unwrap().report, &policy, &ctx, &checks).unwrap();
    assert_eq!(checks.0.lock().unwrap().len(), 2);
    store
        .update(|s| {
            s.report = report.clone();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        build_plan(&report, &report.selected, &snapshot, &policy, now)
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert!(store.load().unwrap().operations.is_empty());
    let selected_supported = dispatch(
        Request {
            protocol_version: 1,
            request_id: "select-supported-fixture".into(),
            action: Action::Selection,
            payload: serde_json::json!({"selected":[app.id]}),
        },
        &paths,
    )
    .unwrap();
    assert!(selected_supported.error.is_none());
    report = store.load().unwrap().report;
    let plan = build_plan(&report, &report.selected, &snapshot, &policy, now).unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].kind, StepKind::TelegramReplace);
    assert!(
        !plan.steps[0]
            .targets
            .contains(&unsupported.components[0].id)
    );
    validate_plan(&plan, &report, &policy, now).unwrap();
    let jobs = Jobs::default();
    let engine = d.path().join("fixture-engine");
    fs::write(&engine, "fixture worker bytes").unwrap();
    let id = submit_with_backend(&plan, &paths, &jobs, &engine, now).unwrap();
    assert_eq!(store.load().unwrap().operations[&id].phase, Phase::Queued);
    let panel_plan_digest = plan.digest.clone();
    // Drop the frontend observation. The independent worker and pinned report remain.
    drop(selected);
    let payload = d.path().join("payload/Telegram");
    fs::create_dir_all(&payload).unwrap();
    fs::write(payload.join("Telegram"), elf(2)).unwrap();
    fs::write(payload.join("Updater"), elf(3)).unwrap();
    let archive = d.path().join("telegram.tar.xz");
    assert!(
        std::process::Command::new("/usr/bin/tar")
            .args(["--format=ustar", "-cJf"])
            .arg(&archive)
            .arg("-C")
            .arg(payload.parent().unwrap())
            .arg("Telegram")
            .status()
            .unwrap()
            .success()
    );
    ctx.http = HttpClient::with_transport(Arc::new(Http(fs::read(archive).unwrap())));
    let handle = JobHandle::load(&id, &paths).unwrap();
    let (ready, gates) = mpsc::channel();
    let worker = std::thread::spawn(move || -> AppResult<()> {
        let server = ControlServer::start(handle.clone())?;
        let execution = ExecutionContext::new(ctx, handle.record().operation.id.clone(), &handle)
            .with_report(handle.record().report)
            .with_processes(Arc::new(Processes));
        let stage = stage_telegram(&handle.record().plan.steps[0], &execution)?;
        let mut revised = handle.record().plan;
        revised.steps[0].downloads[0].expected_sha256 = Some(stage.archive_hash.clone());
        revised.digest = plan_digest(&revised);
        handle.revise(revised)?;
        let gate = handle.gate(
            "Approve fixture artifact",
            vec![DecisionValue::Approve, DecisionValue::Reject],
            None,
        )?;
        ready.send(gate.clone()).unwrap();
        assert_eq!(server.wait(&gate)?, DecisionValue::Approve);
        handle.clear_gate()?;
        let result = activate_telegram(&stage, &execution)?;
        handle.results(vec![result])?;
        handle.event(Phase::Completed, "Verified fixture update", None)
    });
    let gate = gates
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert_ne!(gate.digest, panel_plan_digest);
    assert_eq!(
        respond(
            &id,
            &UserDecision {
                question_id: gate.question_id.clone(),
                digest: panel_plan_digest,
                value: DecisionValue::Approve
            },
            &paths
        )
        .unwrap_err()
        .code,
        ErrorCode::PlanChanged
    );
    assert_eq!(installed_version(&root).unwrap().unwrap().raw, "1.0");
    respond(
        &id,
        &UserDecision {
            question_id: gate.question_id,
            digest: gate.digest,
            value: DecisionValue::Approve,
        },
        &paths,
    )
    .unwrap();
    worker.join().unwrap().unwrap();
    let state = Store::new(paths.clone()).load().unwrap();
    assert_eq!(state.operations[&id].phase, Phase::Completed);
    assert_eq!(state.operations[&id].results[0].outcome, Outcome::Updated);
    assert_eq!(
        state
            .report
            .apps
            .iter()
            .find(|a| a.id == app.id)
            .unwrap()
            .components[0]
            .installed
            .as_ref()
            .unwrap()
            .raw,
        "2.0"
    );
    assert_eq!(installed_version(&root).unwrap().unwrap().raw, "2.0");
    assert_eq!(
        fs::read(profile.join("account")).unwrap(),
        b"preserve fixture account"
    );
    let observation = dispatch(
        Request {
            protocol_version: 1,
            request_id: "observe-fixture".into(),
            action: Action::Operation,
            payload: serde_json::json!({"operation_id":id}),
        },
        &paths,
    )
    .unwrap();
    assert_eq!(
        observation.result.unwrap()["operation"]["id"],
        serde_json::to_value(&id).unwrap()
    );
    assert_eq!(jobs.0.lock().unwrap().len(), 1);
    assert_eq!(
        jobs::load_job(&id, &paths).unwrap().operation.results.len(),
        1
    );
}
