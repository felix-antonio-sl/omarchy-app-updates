mod common;
use omarchy_app_updates::{
    domain::*,
    helper::{DecisionValue, QuestionId},
    jobs::{
        control::{DecisionGate, UserDecision},
        launch::{JobBackend, LaunchSpec, UnitState, submit_with_backend},
        recovery::recover_with_backend,
    },
    store::Store,
};
use std::{
    fs,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct Backend {
    launches: Mutex<Vec<LaunchSpec>>,
    state: Mutex<Option<UnitState>>,
}
impl JobBackend for Backend {
    fn launch(&self, spec: &LaunchSpec) -> AppResult<()> {
        self.launches.lock().unwrap().push(spec.clone());
        Ok(())
    }
    fn state(&self, _: &str) -> AppResult<UnitState> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(UnitState::Active))
    }
}
#[test]
fn accepted_job_is_queued_and_pins_its_executable_without_panel_lifetime() {
    let d = tempfile::tempdir().unwrap();
    let (plan, _, paths) = common::plan(d.path());
    let backend = Arc::new(Backend::default());
    let mut state = State {
        report: common::fixture(d.path()).0,
        ..State::default()
    };
    state.config.selected = state.report.selected.clone();
    state.config.initialized = true;
    state.plans.insert(plan.id.clone(), plan.clone());
    Store::new(paths.clone()).save(&state).unwrap();
    let source = d.path().join("engine");
    fs::write(&source, b"first executable").unwrap();
    let id = submit_with_backend(&plan, &paths, backend.as_ref(), &source, 100).unwrap();
    let saved = Store::new(paths.clone()).load().unwrap();
    assert_eq!(saved.operations[&id].phase, Phase::Queued);
    assert_ne!(saved.operations[&id].phase, Phase::Completed);
    let launches = backend.launches.lock().unwrap();
    assert_eq!(launches.len(), 1);
    assert!(launches[0].unit.starts_with("omarchy-app-updates-"));
    let pinned = launches[0].executable.clone();
    drop(launches);
    fs::write(&source, b"new executable after plugin reload").unwrap();
    assert_eq!(fs::read(pinned).unwrap(), b"first executable");
    assert_eq!(
        submit_with_backend(&plan, &paths, backend.as_ref(), &source, 100)
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
}
#[test]
fn a_stale_or_foreign_decision_cannot_confirm_the_latest_plan() {
    let gate = DecisionGate {
        operation_id: "op-a".into(),
        question_id: QuestionId("question-new".into()),
        digest: "new-digest".into(),
        expires_at: 300,
        choices: vec![DecisionValue::Approve, DecisionValue::Reject],
    };
    let valid = UserDecision {
        question_id: gate.question_id.clone(),
        digest: gate.digest.clone(),
        value: DecisionValue::Approve,
    };
    assert!(gate.validate(&"op-a".into(), &valid, 100).is_ok());
    assert!(gate.validate(&"op-b".into(), &valid, 100).is_err());
    assert!(gate.validate(&"op-a".into(), &valid, 300).is_err());
    let mut stale = valid;
    stale.digest = "previous-digest".into();
    assert!(gate.validate(&"op-a".into(), &stale, 100).is_err());
}
#[test]
fn worker_receipts_do_not_replace_another_operation_or_new_selection() {
    let d = tempfile::tempdir().unwrap();
    let (_, _, paths) = common::plan(d.path());
    let store = Store::new(paths.clone());
    store.save(&State::default()).unwrap();
    store
        .update(|s| {
            s.operations.insert(
                "op-a".into(),
                Operation {
                    id: "op-a".into(),
                    plan_digest: "a".into(),
                    phase: Phase::Completed,
                    unit: None,
                    events: vec![],
                    results: vec![],
                },
            );
            Ok(())
        })
        .unwrap();
    store
        .update(|s| {
            s.config.selected = vec!["chosen-by-user".into()];
            s.operations.insert(
                "op-b".into(),
                Operation {
                    id: "op-b".into(),
                    plan_digest: "b".into(),
                    phase: Phase::Cancelled,
                    unit: None,
                    events: vec![],
                    results: vec![],
                },
            );
            Ok(())
        })
        .unwrap();
    store
        .update(|s| {
            s.report.checked_at = Some(200);
            Ok(())
        })
        .unwrap();
    let saved = store.load().unwrap();
    assert_eq!(
        saved.operations[&OperationId("op-a".into())].phase,
        Phase::Completed
    );
    assert_eq!(
        saved.operations[&OperationId("op-b".into())].phase,
        Phase::Cancelled
    );
    assert_eq!(saved.config.selected, vec![AppId("chosen-by-user".into())]);
}
#[test]
fn recovery_checks_unit_state_and_never_invents_completed_results() {
    let d = tempfile::tempdir().unwrap();
    let (plan, _, paths) = common::plan(d.path());
    let backend = Backend::default();
    let source = d.path().join("engine");
    fs::write(&source, b"engine").unwrap();
    let mut state = State {
        report: common::fixture(d.path()).0,
        ..State::default()
    };
    state.config.selected = state.report.selected.clone();
    state.config.initialized = true;
    Store::new(paths.clone()).save(&state).unwrap();
    let id = submit_with_backend(&plan, &paths, &backend, &source, 100).unwrap();
    *backend.state.lock().unwrap() = Some(UnitState::Inactive);
    let result = recover_with_backend(&paths, &backend, 200).unwrap();
    let op = result.iter().find(|o| o.id == id).unwrap();
    assert_eq!(op.phase, Phase::Failed);
    assert!(op.results.iter().all(|r| r.outcome == Outcome::Failed));
}

#[test]
fn failed_sources_retain_stale_cache_and_checks_limit_concurrency() {
    use omarchy_app_updates::{
        jobs::refresh::refresh_with_provider,
        providers::{Provider, ProviderContext},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Checks {
        active: AtomicUsize,
        max: AtomicUsize,
    }
    impl Provider for Checks {
        fn check(&self, t: &Target, _: &ProviderContext) -> AppResult<Check> {
            let count = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max.fetch_max(count, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(25));
            self.active.fetch_sub(1, Ordering::SeqCst);
            Err(AppError::new(
                ErrorCode::NetworkError,
                format!("{} offline", t.id),
            ))
        }
    }
    let d = tempfile::tempdir().unwrap();
    let (mut report, _, policy, paths) = common::fixture(d.path());
    for n in 0..6 {
        let mut app = report.apps[0].clone();
        app.id = format!("app{n}").into();
        app.components[0].id = format!("target{n}").into();
        report.apps.push(app);
    }
    let backend = Checks {
        active: AtomicUsize::new(0),
        max: AtomicUsize::new(0),
    };
    let checked =
        refresh_with_provider(&report, &policy, &ProviderContext::new(paths), &backend).unwrap();
    assert!(checked.checks[0].stale);
    assert_eq!(checked.checks[0].available, report.checks[0].available);
    assert_eq!(checked.checks.len(), 7);
    assert_eq!(backend.max.load(Ordering::SeqCst), 2);
}
#[test]
fn worker_execution_uses_a_frozen_report_after_inventory_changes() {
    use omarchy_app_updates::{
        execution::ExecutionContext, helper::SilentSink, providers::ProviderContext,
    };
    let d = tempfile::tempdir().unwrap();
    let (report, _, _, paths) = common::fixture(d.path());
    let ctx = ExecutionContext::new(
        ProviderContext::new(paths.clone()),
        "frozen".into(),
        &SilentSink,
    )
    .with_report(report.clone());
    Store::new(paths).save(&State::default()).unwrap();
    assert_eq!(ctx.report().unwrap().apps[0].name, report.apps[0].name);
}

#[test]
fn private_decision_socket_checks_gate_and_survives_a_disconnected_panel() {
    use omarchy_app_updates::jobs::{
        self,
        control::{ControlServer, respond},
    };
    let d = tempfile::tempdir().unwrap();
    let (plan, _, mut paths) = common::plan(d.path());
    paths.runtime_dir = d.path().join("run");
    let backend = Backend::default();
    let source = d.path().join("engine");
    fs::write(&source, b"engine").unwrap();
    let state = State {
        report: common::fixture(d.path()).0,
        ..State::default()
    };
    Store::new(paths.clone()).save(&state).unwrap();
    let id = submit_with_backend(&plan, &paths, &backend, &source, 100).unwrap();
    let job = jobs::worker::JobHandle::load(&id, &paths).unwrap();
    let server = ControlServer::start(job.clone()).unwrap();
    let question = job
        .gate(
            "approved scope",
            vec![DecisionValue::Approve, DecisionValue::Reject],
            None,
        )
        .unwrap();
    let decision = UserDecision {
        question_id: question.question_id.clone(),
        digest: question.digest.clone(),
        value: DecisionValue::Approve,
    };
    let mut invalid = decision.clone();
    invalid.digest = "old digest".into();
    assert_eq!(
        respond(&id, &invalid, &paths).unwrap_err().code,
        ErrorCode::PlanChanged
    );
    // Closing an unrelated panel/stream does not change the unit or operation.
    drop(std::os::unix::net::UnixStream::connect(server.path()).unwrap());
    respond(&id, &decision, &paths).unwrap();
    assert_eq!(server.wait(&question).unwrap(), DecisionValue::Approve);
    assert!(respond(&id, &decision, &paths).is_err());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(server.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let sock = server.path().to_path_buf();
    drop(server);
    assert!(!sock.exists());
}
#[test]
fn cli_exposes_engine_metadata_and_does_not_ignore_worker_arguments() {
    let executable = env!("CARGO_BIN_EXE_app-updates");
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: omarchy_app_updates::engine::EngineMetadata =
        serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(metadata.protocol, 1);
    assert_eq!(metadata.source_digest.len(), 64);
    let invalid = std::process::Command::new(executable)
        .args(["--worker", "../../bad"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stdout).contains("invalid_input"));
}

#[test]
fn recovery_cotes_actual_versions_and_waits_when_unit_state_is_unknown() {
    use omarchy_app_updates::jobs::recovery::{RecoveryVerifier, recover_with_verifier};
    struct Verifier;
    impl RecoveryVerifier for Verifier {
        fn installed(
            &self,
            _: &Target,
            _: &omarchy_app_updates::paths::Paths,
        ) -> AppResult<Option<Version>> {
            Ok(Some(Version::arch("2.0-1")))
        }
    }
    let d = tempfile::tempdir().unwrap();
    let (plan, _, paths) = common::plan(d.path());
    let backend = Backend::default();
    let source = d.path().join("engine");
    fs::write(&source, b"engine").unwrap();
    let state = State {
        report: common::fixture(d.path()).0,
        ..State::default()
    };
    Store::new(paths.clone()).save(&state).unwrap();
    let id = submit_with_backend(&plan, &paths, &backend, &source, 100).unwrap();
    *backend.state.lock().unwrap() = Some(UnitState::Unknown);
    assert_eq!(
        recover_with_verifier(&paths, &backend, &Verifier, 200).unwrap()[0].phase,
        Phase::Queued
    );
    *backend.state.lock().unwrap() = Some(UnitState::Inactive);
    let ops = recover_with_verifier(&paths, &backend, &Verifier, 200).unwrap();
    let op = ops.iter().find(|o| o.id == id).unwrap();
    assert_eq!(op.phase, Phase::Failed);
    assert_eq!(op.results[0].new_version.as_ref().unwrap().raw, "2.0-1");
    assert_eq!(op.results[0].outcome, Outcome::Failed);
}
#[test]
fn administrative_metadata_is_checked_before_authorization() {
    use omarchy_app_updates::engine::{compatible, metadata};
    let mut actual = metadata();
    actual.source_digest = "other build".into();
    assert_eq!(
        compatible(&actual).unwrap_err().code,
        ErrorCode::NeedsEnginePreparation
    );
    actual = metadata();
    actual.alpm_runtime = "999.0.0".into();
    assert_eq!(
        compatible(&actual).unwrap_err().code,
        ErrorCode::NeedsEnginePreparation
    );
}
#[test]
fn cancelling_an_owned_preparation_command_does_not_leave_a_daemon() {
    use omarchy_app_updates::io::command::{CommandRunner, CommandSpec};
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut spec = CommandSpec::new("/usr/bin/sleep").args(["30".into()]);
    spec.cancellation = Some(cancelled.clone());
    let worker = std::thread::spawn(move || CommandRunner::default().run(&spec));
    std::thread::sleep(std::time::Duration::from_millis(80));
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        worker.join().unwrap().unwrap_err().code,
        ErrorCode::AuthCancelled
    );
}
#[test]
#[ignore = "creates a transient USER systemd fixture unit only; no package or app update"]
fn real_user_unit_pins_a_fixture_worker_and_outlives_the_panel() {
    use omarchy_app_updates::jobs::launch::SystemdJobs;
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let (plan, _, paths) = common::plan(d.path());
    let source = d.path().join("fixture-engine");
    let receipt = d.path().join("fixture-receipt");
    assert!(!receipt.exists());
    let script = format!(
        "#!/bin/sh\nsleep 2\nprintf 'fixture completed' > '{}'\n",
        receipt.display()
    );
    fs::write(&source, script).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    let state = State {
        report: common::fixture(d.path()).0,
        ..State::default()
    };
    Store::new(paths.clone()).save(&state).unwrap();
    let id = submit_with_backend(&plan, &paths, &SystemdJobs, &source, 100).unwrap();
    fs::write(&source, "#!/bin/sh\nexit 1\n").unwrap();
    let unit = Store::new(paths.clone()).load().unwrap().operations[&id]
        .unit
        .clone()
        .unwrap();
    assert_eq!(SystemdJobs.state(&unit).unwrap(), UnitState::Active);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !receipt.exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(fs::read_to_string(receipt).unwrap(), "fixture completed");
    // Dispatch alone is not a verified update receipt.
    assert_eq!(
        Store::new(paths).load().unwrap().operations[&id].phase,
        Phase::Queued
    );
}

#[test]
#[ignore = "runs the actual coordinator on an EMPTY fixture plan in a transient USER unit"]
fn actual_worker_completes_an_empty_plan_and_records_monotonic_receipts() {
    use omarchy_app_updates::{jobs::launch::SystemdJobs, planner::build_plan};
    let d = tempfile::tempdir().unwrap();
    let (mut report, snapshot, policy, mut paths) = common::fixture(d.path());
    paths.runtime_dir = d.path().join("run");
    report.apps.clear();
    report.checks.clear();
    report.selected.clear();
    let now = omarchy_app_updates::now();
    let plan = build_plan(&report, &[], &snapshot, &policy, now).unwrap();
    let mut state = State {
        report,
        ..State::default()
    };
    state.config.initialized = true;
    Store::new(paths.clone()).save(&state).unwrap();
    let id = submit_with_backend(
        &plan,
        &paths,
        &SystemdJobs,
        std::path::Path::new(env!("CARGO_BIN_EXE_app-updates")),
        now,
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let state = Store::new(paths.clone()).load().unwrap();
        if state.operations[&id].phase == Phase::Completed {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "{:?}",
            state.operations[&id]
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let job = omarchy_app_updates::jobs::load_job(&id, &paths).unwrap();
    assert!(job.operation.results.is_empty());
    assert!(job.pending.is_none());
    assert_eq!(job.operation.events[0].seq, 1);
}

#[test]
fn state_backup_replacement_cannot_overwrite_a_hardlinked_user_file() {
    let d = tempfile::tempdir().unwrap();
    let (_, _, paths) = common::plan(d.path());
    let store = Store::new(paths.clone());
    store.save(&State::default()).unwrap();
    let protected = d.path().join("personal-file");
    fs::write(&protected, b"preserve me").unwrap();
    fs::hard_link(&protected, paths.state.join("state.json.previous")).unwrap();
    store
        .update(|s| {
            s.report.checked_at = Some(321);
            Ok(())
        })
        .unwrap();
    assert_eq!(fs::read(&protected).unwrap(), b"preserve me");
}

#[test]
fn interrupted_job_with_missing_receipt_updates_the_visible_operation() {
    let d = tempfile::tempdir().unwrap();
    let (plan, _, paths) = common::plan(d.path());
    let backend = Backend::default();
    let source = d.path().join("engine");
    fs::write(&source, b"engine").unwrap();
    let state = State {
        report: common::fixture(d.path()).0,
        ..State::default()
    };
    Store::new(paths.clone()).save(&state).unwrap();
    let id = submit_with_backend(&plan, &paths, &backend, &source, 100).unwrap();
    fs::remove_file(
        omarchy_app_updates::jobs::job_directory(&id, &paths)
            .unwrap()
            .join("job.json"),
    )
    .unwrap();
    *backend.state.lock().unwrap() = Some(UnitState::Inactive);
    recover_with_backend(&paths, &backend, 200).unwrap();
    assert_eq!(
        Store::new(paths)
            .load()
            .unwrap()
            .report
            .operation
            .unwrap()
            .phase,
        Phase::Failed
    );
}
