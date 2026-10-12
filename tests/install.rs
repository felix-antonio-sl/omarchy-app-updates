mod common;
use omarchy_app_updates::{
    domain::*,
    helper::SilentSink,
    install::{self, EngineArtifacts, InstallBackend},
    paths::Paths,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
#[derive(Default)]
struct Backend {
    calls: Mutex<Vec<String>>,
    fail: bool,
    fail_activation: bool,
}
impl InstallBackend for Backend {
    fn build(
        &self,
        source: &Path,
        _: &Paths,
        _: &dyn omarchy_app_updates::execution::EventSink,
    ) -> AppResult<EngineArtifacts> {
        self.calls.lock().unwrap().push("build-locked".into());
        if self.fail {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "fixture build failed",
            ));
        }
        Ok(EngineArtifacts {
            engine: source.join("bin/app-updates"),
            helper: source.join("bin/app-updates-helper"),
            metadata: omarchy_app_updates::engine::metadata(),
        })
    }
    fn install_helper(&self, _: &EngineArtifacts, _: &Path, _: &Paths) -> AppResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push("graphical-package-install".into());
        Ok(())
    }
    fn shell(&self, args: &[&str]) -> AppResult<()> {
        self.calls.lock().unwrap().push(args.join(" "));
        if self.fail_activation && args.first() == Some(&"enable") {
            return Err(AppError::new(
                ErrorCode::TransactionFailed,
                "fixture activation failed",
            ));
        }
        Ok(())
    }
    fn remove_helper(&self, _: &Paths) -> AppResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push("remove-owned-package".into());
        Ok(())
    }
}
#[test]
fn activation_failure_restores_the_previous_plugin_and_does_not_claim_installation() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().join("home"));
    let source = source(d.path());
    fs::create_dir_all(&paths.plugin).unwrap();
    fs::write(paths.plugin.join("previous"), b"working version").unwrap();
    let backend = Backend {
        fail_activation: true,
        ..Default::default()
    };
    assert!(install::install_with_backend(&source, &paths, &backend, &SilentSink).is_err());
    assert_eq!(
        fs::read(paths.plugin.join("previous")).unwrap(),
        b"working version"
    );
    assert!(!paths.state.join("installation.json").exists());
}
fn source(home: &Path) -> PathBuf {
    let source = home.join("source");
    fs::create_dir_all(source.join("bin")).unwrap();
    fs::create_dir_all(source.join("ui")).unwrap();
    fs::write(source.join("manifest.json"),r#"{"schemaVersion":1,"id":"korvo.app-updates","version":"0.1.0","kinds":["bar-widget"],"entryPoints":{"barWidget":"ui/BarWidget.qml"}}"#).unwrap();
    fs::write(
        source.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(source.join("Cargo.lock"), "# fixture lock\nversion = 4\n").unwrap();
    fs::write(source.join("ui/BarWidget.qml"), "import QtQuick\nItem {}\n").unwrap();
    fs::write(source.join("bin/app-updates"), b"fixture engine").unwrap();
    fs::write(source.join("bin/app-updates-helper"), b"fixture helper").unwrap();
    source
}
#[test]
fn install_is_atomic_and_reversible_and_keeps_other_widgets_and_profiles() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().join("home"));
    let source = source(d.path());
    let shell = paths.home.join(".config/omarchy/shell.json");
    fs::create_dir_all(shell.parent().unwrap()).unwrap();
    let original = r#"{"bar":{"right":["omarchy.clock","korvo.minsal-vpn"]}}"#;
    fs::write(&shell, original).unwrap();
    let profile = paths.home.join("personal-data");
    fs::write(&profile, b"preserve profile").unwrap();
    let backend = Backend::default();
    let receipt = install::install_with_backend(&source, &paths, &backend, &SilentSink).unwrap();
    assert!(paths.plugin.join("ui/BarWidget.qml").exists());
    assert_eq!(
        fs::read_to_string(receipt.shell_backup.unwrap()).unwrap(),
        original
    );
    assert_eq!(fs::read_to_string(&shell).unwrap(), original);
    assert_eq!(
        backend
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.as_str() == "build-locked")
            .count(),
        1
    );
    assert!(
        backend
            .calls
            .lock()
            .unwrap()
            .contains(&"enable korvo.app-updates".into())
    );
    install::remove_with_backend(&paths, &backend).unwrap();
    assert!(!paths.plugin.exists());
    assert_eq!(fs::read(profile).unwrap(), b"preserve profile");
    assert_eq!(fs::read_to_string(shell).unwrap(), original);
}
#[test]
fn failed_build_or_linked_source_never_replaces_the_live_plugin() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().join("home"));
    let source = source(d.path());
    fs::create_dir_all(&paths.plugin).unwrap();
    fs::write(paths.plugin.join("previous"), b"working version").unwrap();
    let backend = Backend {
        fail: true,
        ..Default::default()
    };
    assert!(install::install_with_backend(&source, &paths, &backend, &SilentSink).is_err());
    assert_eq!(
        fs::read(paths.plugin.join("previous")).unwrap(),
        b"working version"
    );
    std::os::unix::fs::symlink("/etc/passwd", source.join("ui/linked.qml")).unwrap();
    assert_eq!(
        install::validate_source(&source).unwrap_err().code,
        ErrorCode::UnsafeArtifact
    );
}
#[test]
fn missing_lockfile_and_version_mismatch_block_preparation() {
    let d = tempfile::tempdir().unwrap();
    let source = source(d.path());
    fs::remove_file(source.join("Cargo.lock")).unwrap();
    assert!(install::validate_source(&source).is_err());
    assert!(!install::engine_compatible("0.1.0", "0.2.0", 1, 16));
    assert!(!install::engine_compatible("0.1.0", "0.1.0", 2, 16));
    assert!(!install::engine_compatible("0.1.0", "0.1.0", 1, 999));
    assert!(install::engine_compatible("0.1.0", "0.1.0", 1, 16));
}
#[test]
fn active_job_blocks_plugin_replacement_and_removal() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().join("home"));
    let source = source(d.path());
    let store = omarchy_app_updates::store::Store::new(paths.clone());
    store
        .update(|s| {
            s.operations.insert(
                "active".into(),
                Operation {
                    id: "active".into(),
                    plan_digest: "digest".into(),
                    phase: Phase::Installing,
                    unit: Some("fixture.service".into()),
                    events: vec![],
                    results: vec![],
                },
            );
            Ok(())
        })
        .unwrap();
    let backend = Backend::default();
    assert_eq!(
        install::install_with_backend(&source, &paths, &backend, &SilentSink)
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    assert_eq!(
        install::remove_with_backend(&paths, &backend)
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
    assert!(backend.calls.lock().unwrap().is_empty());
}
#[test]
fn staging_parent_links_are_rejected_without_creating_external_directories() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().join("home"));
    let source = source(d.path());
    fs::create_dir_all(&paths.home).unwrap();
    let external = d.path().join("outside");
    fs::create_dir(&external).unwrap();
    std::os::unix::fs::symlink(&external, paths.home.join(".config")).unwrap();
    let backend = Backend::default();
    assert!(install::install_with_backend(&source, &paths, &backend, &SilentSink).is_err());
    assert_eq!(fs::read_dir(external).unwrap().count(), 0);
}
#[test]
fn an_engine_preparation_lease_blocks_new_app_submissions() {
    use omarchy_app_updates::jobs::launch::{
        JobBackend, LaunchSpec, UnitState, submit_with_backend,
    };
    struct Jobs;
    impl JobBackend for Jobs {
        fn launch(&self, _: &LaunchSpec) -> AppResult<()> {
            Err(AppError::new(
                ErrorCode::TransactionFailed,
                "must not launch",
            ))
        }
        fn state(&self, _: &str) -> AppResult<UnitState> {
            Ok(UnitState::Inactive)
        }
    }
    let d = tempfile::tempdir().unwrap();
    let (plan, _, paths) = common::plan(d.path());
    let state = State {
        report: common::fixture(d.path()).0,
        ..Default::default()
    };
    omarchy_app_updates::store::Store::new(paths.clone())
        .save(&state)
        .unwrap();
    let engine = d.path().join("engine");
    fs::write(&engine, b"engine").unwrap();
    let _lease = omarchy_app_updates::jobs::worker::worker_lock(&paths).unwrap();
    assert_eq!(
        submit_with_backend(&plan, &paths, &Jobs, &engine, 100)
            .unwrap_err()
            .code,
        ErrorCode::Busy
    );
}
#[test]
fn bootstrap_package_scope_rejects_unexpected_files_dependencies_or_identity() {
    use omarchy_app_updates::alpm::ArchiveInfo;
    let mut package = ArchiveInfo {
        name: "omarchy-app-updates-helper".into(),
        version: "0.1.0-1".into(),
        architecture: "x86_64".into(),
        files: vec![
            "usr/lib/omarchy-app-updates/app-updates-helper".into(),
            "usr/share/polkit-1/actions/org.korvo.app-updates.policy".into(),
        ],
        depends: vec!["pacman".into(), "flatpak".into(), "libarchive".into()],
    };
    assert!(install::validate_package_contents(&package).is_ok());
    package.files.push("etc/sudoers.d/bypass".into());
    assert!(install::validate_package_contents(&package).is_err());
    package.files.pop();
    package.depends.push("unexpected-extra".into());
    assert!(install::validate_package_contents(&package).is_err());
    package.depends.pop();
    package.name = "other-package".into();
    assert!(install::validate_package_contents(&package).is_err());
}

#[test]
#[ignore = "builds and inspects a user-owned local fixture package with makepkg; never installs it"]
fn native_helper_package_is_limited_to_owned_paths_and_has_no_install_script() {
    let d = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(d.path().join("home"));
    let source = d.path().join("source");
    fs::create_dir_all(source.join("packaging")).unwrap();
    for name in ["PKGBUILD", "org.korvo.app-updates.policy"] {
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("packaging")
                .join(name),
            source.join("packaging").join(name),
        )
        .unwrap();
    }
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("LICENSE"),
        source.join("LICENSE"),
    )
    .unwrap();
    let helper = d.path().join("fixture-helper");
    fs::write(&helper, b"fixture helper executable").unwrap();
    let artifacts = EngineArtifacts {
        engine: helper.clone(),
        helper,
        metadata: omarchy_app_updates::engine::metadata(),
    };
    let package = install::build_helper_package(&artifacts, &source, &paths).unwrap();
    assert!(package.starts_with(&paths.state));
    let output = std::process::Command::new("/usr/bin/bsdtar")
        .args(["-tvf"])
        .arg(package)
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("usr/lib/omarchy-app-updates/app-updates-helper"));
    assert!(!text.contains(".INSTALL"));
    assert!(
        text.lines()
            .all(|l| l.starts_with('-') || l.starts_with('d'))
    );
}
