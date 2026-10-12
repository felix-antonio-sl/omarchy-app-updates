use omarchy_app_updates::{
    domain::*,
    execution::{ExecutionContext, reopen::needs_reopen, steam::*},
    helper::SilentSink,
    paths::Paths,
    providers::{ProviderContext, steam::detect_channel},
    store::Store,
};
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Backend {
    state: SteamState,
    update: bool,
    calls: AtomicUsize,
}
impl SteamBackend for Backend {
    fn state(&self, _: &Path, _: &ExecutionContext) -> AppResult<SteamState> {
        Ok(self.state.clone())
    }
    fn start(&self, root: &Path, _: &str, _: &ExecutionContext) -> AppResult<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.update {
            write_client(root, "2", b"new deployed client");
        }
        Ok(())
    }
    fn wait(&self, _: &Path, _: &str, _: &str, _: &str, _: &ExecutionContext) -> AppResult<()> {
        Ok(())
    }
}
fn write_client(root: &Path, version: &str, binary: &[u8]) {
    fs::create_dir_all(root.join("package")).unwrap();
    fs::create_dir_all(root.join("ubuntu12_32")).unwrap();
    fs::write(root.join("ubuntu12_32/steam"), binary).unwrap();
    fs::write(
        root.join("package/steam_client_ubuntu12.manifest"),
        format!("\"ubuntu12\"{{\"version\"\"{version}\"}}"),
    )
    .unwrap();
    fs::write(
        root.join("package/steam_client_ubuntu12.installed"),
        format!(
            "ubuntu12_32/steam,{};1;{}\n",
            binary.len(),
            crc32fast::hash(binary)
        ),
    )
    .unwrap();
}
fn fixture(
    home: &Path,
    state: SteamState,
    update: bool,
) -> (ProviderContext, PlanStep, Arc<Backend>) {
    let paths = Paths::for_home(home.into());
    let root = home.join(".local/share/Steam");
    write_client(&root, "1", b"old deployed client");
    fs::create_dir_all(root.join("steamapps/common/game")).unwrap();
    fs::create_dir_all(root.join("userdata/account")).unwrap();
    fs::write(root.join("steamapps/common/game/data"), b"unchanged game").unwrap();
    fs::write(root.join("userdata/account/data"), b"unchanged account").unwrap();
    let target = Target {
        id: "steam:client".into(),
        locator: Locator::SteamClient {
            root,
            channel: "ubuntu12".into(),
        },
        installed: Some(Version::product("1")),
    };
    let mut saved = State::default();
    saved.report.apps.push(App {
        id: "repo:steam".into(),
        name: "Steam".into(),
        desktop_ids: vec![],
        components: vec![target.clone()],
        suggested: true,
    });
    saved.report.checks.push(Check {
        target_id: target.id.clone(),
        installed: target.installed,
        available: Some(Version::product("2")),
        published: None,
        checked_at: 100,
        state: CheckState::UpdateAvailable,
        stale: false,
        source: None,
        details: vec![],
    });
    Store::new(paths.clone()).save(&saved).unwrap();
    let step = PlanStep {
        kind: StepKind::SteamUpdate,
        targets: vec![target.id],
        downloads: vec![Download {
            url: omarchy_app_updates::providers::steam::manifest_url("ubuntu12").unwrap(),
            expected_sha256: None,
            bytes: None,
        }],
        package_changes: vec![],
        reopen: vec![],
        requires_auth: false,
        aur_recipe: None,
        flatpak_changes: vec![],
        built_artifacts: vec![],
        build_dependencies: vec![],
    };
    (
        ProviderContext::new(paths),
        step,
        Arc::new(Backend {
            state,
            update,
            calls: AtomicUsize::new(0),
        }),
    )
}
#[test]
fn games_running_or_unknown_state_never_launch_or_signal_the_client() {
    for state in [
        SteamState::GamesRunning,
        SteamState::Unknown,
        SteamState::ClientRunning,
    ] {
        let d = tempfile::tempdir().unwrap();
        let (ctx, step, backend) = fixture(d.path(), state, true);
        let exec = ExecutionContext::new(ctx, "op".into(), &SilentSink).with_steam(backend.clone());
        assert_eq!(run_steam(&step, &exec).unwrap_err().code, ErrorCode::Busy);
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn closed_client_updates_its_component_and_keeps_games_and_userdata() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, step, backend) = fixture(d.path(), SteamState::Closed, true);
    let exec = ExecutionContext::new(ctx, "op".into(), &SilentSink).with_steam(backend.clone());
    let result = run_steam(&step, &exec).unwrap();
    assert_eq!(result.target_id.0, "steam:client");
    assert_eq!(result.outcome, Outcome::Updated);
    assert_eq!(result.old_version.unwrap().raw, "1");
    assert_eq!(result.new_version.unwrap().raw, "2");
    assert_eq!(
        fs::read(
            d.path()
                .join(".local/share/Steam/steamapps/common/game/data")
        )
        .unwrap(),
        b"unchanged game"
    );
    assert_eq!(
        fs::read(d.path().join(".local/share/Steam/userdata/account/data")).unwrap(),
        b"unchanged account"
    );
}
#[test]
fn successful_submission_and_a_downloaded_manifest_are_not_a_client_update() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, step, backend) = fixture(d.path(), SteamState::Closed, false);
    let exec = ExecutionContext::new(ctx, "op".into(), &SilentSink).with_steam(backend);
    assert!(run_steam(&step, &exec).is_err());
    let root = d.path().join(".local/share/Steam");
    let inventory = omarchy_app_updates::providers::telegram::binary_hash(
        &root.join("package/steam_client_ubuntu12.installed"),
    )
    .unwrap();
    fs::write(
        root.join("package/steam_client_ubuntu12.manifest"),
        "\"ubuntu12\"{\"version\"\"2\"}",
    )
    .unwrap();
    assert!(verify_client_installation(&root, "ubuntu12", "2", Some(&inventory)).is_err());
}
#[test]
fn channel_marker_is_preserved_and_changed_channels_require_another_plan() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, step, backend) = fixture(d.path(), SteamState::Closed, true);
    let root = d.path().join(".local/share/Steam");
    assert_eq!(detect_channel(&root).unwrap(), "ubuntu12");
    fs::write(root.join("package/beta"), "publicbeta").unwrap();
    assert_eq!(detect_channel(&root).unwrap(), "ubuntu12_beta");
    let exec = ExecutionContext::new(ctx, "op".into(), &SilentSink).with_steam(backend.clone());
    assert_eq!(
        run_steam(&step, &exec).unwrap_err().code,
        ErrorCode::PlanChanged
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    fs::write(root.join("package/beta"), "otherchannel").unwrap();
    assert!(detect_channel(&root).is_err());
}
#[test]
fn deleted_executable_requires_reopen_without_closing_its_live_process() {
    use omarchy_app_updates::execution::reopen::{LinuxProcesses, ProcessBackend};
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("browser");
    fs::copy("/usr/bin/sleep", &path).unwrap();
    let mut child = std::process::Command::new(&path).arg("60").spawn().unwrap();
    let mut captured = vec![];
    for _ in 0..30 {
        captured = LinuxProcesses.running(std::slice::from_ref(&path)).unwrap();
        if !captured.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(captured.len(), 1);
    fs::rename(&path, d.path().join("previous")).unwrap();
    fs::write(&path, b"new binary").unwrap();
    let app = App {
        id: "repo:browser".into(),
        name: "Browser".into(),
        desktop_ids: vec![],
        components: vec![],
        suggested: true,
    };
    let result = needs_reopen(&app, &captured).unwrap();
    let still_running = child.try_wait().unwrap().is_none();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(result);
    assert!(still_running);
}

#[test]
#[ignore = "host read-only Steam core installation verification"]
fn host_steam_core_matches_its_public_installed_inventory() {
    let root =
        std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".local/share/Steam");
    let channel = detect_channel(&root).unwrap();
    let version = omarchy_app_updates::providers::steam::manifest_version(
        &fs::read(root.join("package/steam_client_ubuntu12.manifest")).unwrap(),
        &channel,
    )
    .unwrap();
    assert_eq!(
        verify_client_installation(&root, &channel, &version.raw, None)
            .unwrap()
            .raw,
        version.raw
    );
}
