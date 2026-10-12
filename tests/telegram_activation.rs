use omarchy_app_updates::{
    domain::*,
    execution::{ExecutionContext, reopen::*, telegram::*},
    helper::SilentSink,
    io::{command::*, http::*},
    paths::Paths,
    providers::{
        ProviderContext,
        telegram::{TelegramReceipt, binary_hash},
    },
    store::Store,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
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
struct Commands;
impl CommandBackend for Commands {
    fn run(&self, spec: &CommandSpec) -> AppResult<Captured> {
        assert!(matches!(
            spec.program.to_str(),
            Some("/usr/bin/readelf" | "/usr/bin/ldconfig")
        ));
        Ok(Captured {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}
#[derive(Default)]
struct Processes {
    running: Mutex<Vec<RunningApp>>,
    closed: Mutex<Vec<u32>>,
}
impl ProcessBackend for Processes {
    fn running(&self, _: &[PathBuf]) -> AppResult<Vec<RunningApp>> {
        Ok(self.running.lock().unwrap().clone())
    }
    fn matches(&self, p: &RunningApp) -> AppResult<bool> {
        Ok(self.running.lock().unwrap().iter().any(|r| r == p))
    }
    fn close(&self, p: &RunningApp) -> AppResult<()> {
        if !self.matches(p)? {
            return Err(AppError::new(ErrorCode::PlanChanged, "PID reused"));
        }
        self.closed.lock().unwrap().push(p.pid);
        self.running.lock().unwrap().retain(|r| r != p);
        Ok(())
    }
    fn launch(&self, _: &App, _: &Paths) -> AppResult<()> {
        Ok(())
    }
}
struct FailSwap {
    count: Mutex<usize>,
}
impl ActivationBackend for FailSwap {
    fn rename(&self, from: &Path, to: &Path) -> AppResult<()> {
        let mut count = self.count.lock().unwrap();
        *count += 1;
        if *count == 4 {
            return Err(AppError::new(
                ErrorCode::IoError,
                "simulated activation failure",
            ));
        }
        fs::rename(from, to)?;
        Ok(())
    }
}
fn elf(marker: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    bytes[16] = 2;
    bytes[18] = 62;
    bytes[48] = marker;
    bytes
}
fn payload(home: &Path, bad: &str) -> Vec<u8> {
    let tree = home.join("payload");
    fs::create_dir_all(tree.join("Telegram")).unwrap();
    fs::write(tree.join("Telegram/Telegram"), elf(2)).unwrap();
    if bad == "symlink" {
        symlink("../../outside", tree.join("Telegram/Updater")).unwrap();
    } else {
        fs::write(tree.join("Telegram/Updater"), elf(3)).unwrap();
    }
    let archive = home.join("telegram.tar.xz");
    let mut cmd = Command::new("/usr/bin/tar");
    cmd.arg("--format=ustar")
        .arg("-cJf")
        .arg(&archive)
        .arg("-C")
        .arg(&tree);
    if bad == "traversal" {
        cmd.arg("--transform=s|Telegram/Telegram|../../outside|");
    }
    cmd.arg("Telegram");
    assert!(cmd.status().unwrap().success());
    fs::read(archive).unwrap()
}
fn fixture(
    home: &Path,
    bad: &str,
    version: &str,
) -> (ProviderContext, App, PlanStep, Arc<Processes>) {
    let paths = Paths::for_home(home.into());
    let root = home.join(".local/opt/telegram-desktop");
    fs::create_dir_all(&root).unwrap();
    for (name, mark) in [("Telegram", 1), ("Updater", 0)] {
        fs::write(root.join(name), elf(mark)).unwrap();
        fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::create_dir_all(root.join("tdata")).unwrap();
    fs::write(root.join("tdata/account"), b"portable account").unwrap();
    let tdata = home.join(".local/share/TelegramDesktop/tdata");
    fs::create_dir_all(&tdata).unwrap();
    fs::write(tdata.join("account"), b"existing account").unwrap();
    let bin = home.join(".local/bin");
    fs::create_dir_all(&bin).unwrap();
    symlink(root.join("Telegram"), bin.join("telegram-desktop")).unwrap();
    fs::write(
        root.join(".app-updates-version.json"),
        serde_json::to_vec(&TelegramReceipt {
            version: "1.0".into(),
            binary_sha256: binary_hash(&root.join("Telegram")).unwrap(),
        })
        .unwrap(),
    )
    .unwrap();
    let target = Target {
        id: "telegram:fixture".into(),
        locator: Locator::TelegramLocal { root },
        installed: Some(Version::product("1.0")),
    };
    let app = App {
        id: "telegram:fixture".into(),
        name: "Telegram Desktop".into(),
        desktop_ids: vec!["telegram.desktop".into()],
        components: vec![target.clone()],
        suggested: true,
    };
    let mut state = State::default();
    state.report.apps = vec![app.clone()];
    Store::new(paths.clone()).save(&state).unwrap();
    let mut ctx = ProviderContext::new(paths);
    ctx.http = HttpClient::with_transport(Arc::new(Http(payload(home, bad))));
    ctx.commands = CommandRunner::with_backend(Arc::new(Commands));
    let step = PlanStep {
        kind: StepKind::TelegramReplace,
        targets: vec![target.id],
        package_changes: vec![],
        downloads: vec![Download {
            url: format!("https://td.telegram.org/linux-x64/td-setup-linux-x64-{version}.tar.xz"),
            expected_sha256: None,
            bytes: None,
        }],
        requires_auth: false,
        reopen: vec![],
        aur_recipe: None,
        flatpak_changes: vec![],
        build_dependencies: vec![],
        built_artifacts: vec![],
    };
    (ctx, app, step, Arc::new(Processes::default()))
}
#[test]
fn telegram_preserves_profile_launcher_and_previous_executables() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, _, step, processes) = fixture(d.path(), "", "2.0");
    let root = d.path().join(".local/opt/telegram-desktop");
    let previous = binary_hash(&root.join("Telegram")).unwrap();
    let link = fs::read_link(d.path().join(".local/bin/telegram-desktop")).unwrap();
    let exec = ExecutionContext::new(ctx, "tg-op".into(), &SilentSink).with_processes(processes);
    let stage = stage_telegram(&step, &exec).unwrap();
    assert!(!stage.vendor_signature_verified);
    let result = activate_telegram(&stage, &exec).unwrap();
    assert_eq!(result.outcome, Outcome::Updated);
    assert_eq!(
        fs::read(root.join("tdata/account")).unwrap(),
        b"portable account"
    );
    assert_eq!(
        fs::read(d.path().join(".local/share/TelegramDesktop/tdata/account")).unwrap(),
        b"existing account"
    );
    assert_eq!(
        fs::read_link(d.path().join(".local/bin/telegram-desktop")).unwrap(),
        link
    );
    assert_eq!(
        binary_hash(&stage.previous.join("Telegram")).unwrap(),
        previous
    );
    assert_eq!(
        omarchy_app_updates::providers::telegram::installed_version(&root)
            .unwrap()
            .unwrap()
            .raw,
        "2.0"
    );
}
#[test]
fn telegram_rejects_traversal_and_external_links_before_activation() {
    for bad in ["traversal", "symlink"] {
        let d = tempfile::tempdir().unwrap();
        let (ctx, _, step, processes) = fixture(d.path(), bad, "2.0");
        let exec =
            ExecutionContext::new(ctx, "tg-op".into(), &SilentSink).with_processes(processes);
        assert_eq!(
            stage_telegram(&step, &exec).unwrap_err().code,
            ErrorCode::UnsafeArtifact
        );
        assert!(!d.path().join("outside").exists());
    }
}
#[test]
fn running_telegram_requires_a_decision_and_equal_version_never_closes_it() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, _, step, processes) = fixture(d.path(), "", "2.0");
    let running = RunningApp {
        pid: 42,
        exe: d.path().join(".local/opt/telegram-desktop/Telegram"),
        start_ticks: 100,
        binary: BinaryIdentity {
            device: 1,
            inode: 2,
            sha256: "a".repeat(64),
        },
    };
    processes.running.lock().unwrap().push(running);
    let exec = ExecutionContext::new(ctx.clone(), "tg-op".into(), &SilentSink)
        .with_processes(processes.clone());
    let stage = stage_telegram(&step, &exec).unwrap();
    assert_eq!(
        activate_telegram(&stage, &exec).unwrap_err().code,
        ErrorCode::Busy
    );
    assert!(processes.closed.lock().unwrap().is_empty());
    let mut current = step;
    current.downloads[0].url =
        "https://td.telegram.org/linux-x64/td-setup-linux-x64-1.0.tar.xz".into();
    let stage = stage_telegram(&current, &exec).unwrap();
    assert_eq!(
        activate_telegram(&stage, &exec).unwrap().outcome,
        Outcome::Current
    );
    assert!(processes.closed.lock().unwrap().is_empty());
}
#[test]
fn activation_failure_restores_both_previous_binaries_and_receipt() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, _, step, processes) = fixture(d.path(), "", "2.0");
    let root = d.path().join(".local/opt/telegram-desktop");
    let old = fs::read(root.join("Telegram")).unwrap();
    let updater = fs::read(root.join("Updater")).unwrap();
    let receipt = fs::read(root.join(".app-updates-version.json")).unwrap();
    let exec = ExecutionContext::new(ctx, "tg-op".into(), &SilentSink)
        .with_processes(processes)
        .with_activation(Arc::new(FailSwap {
            count: Mutex::new(0),
        }));
    let stage = stage_telegram(&step, &exec).unwrap();
    assert!(activate_telegram(&stage, &exec).is_err());
    assert_eq!(fs::read(root.join("Telegram")).unwrap(), old);
    assert_eq!(fs::read(root.join("Updater")).unwrap(), updater);
    assert_eq!(
        fs::read(root.join(".app-updates-version.json")).unwrap(),
        receipt
    );
}
#[test]
fn reused_pid_never_receives_a_reopen_signal() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, app, _, processes) = fixture(d.path(), "", "2.0");
    let original = RunningApp {
        pid: 42,
        exe: d.path().join(".local/opt/telegram-desktop/Telegram"),
        start_ticks: 100,
        binary: BinaryIdentity {
            device: 1,
            inode: 2,
            sha256: "a".repeat(64),
        },
    };
    let mut reused = original.clone();
    reused.start_ticks = 101;
    processes.running.lock().unwrap().push(reused);
    let exec =
        ExecutionContext::new(ctx, "tg-op".into(), &SilentSink).with_processes(processes.clone());
    let approval = ReopenApproval {
        operation_id: exec.operation_id.clone(),
        app_id: app.id.clone(),
        processes: vec![original],
    };
    assert_eq!(
        request_reopen(&app, &approval, &exec).unwrap_err().code,
        ErrorCode::PlanChanged
    );
    assert!(processes.closed.lock().unwrap().is_empty());
}

#[test]
fn changed_installation_directory_is_rejected_before_any_executable_swap() {
    let d = tempfile::tempdir().unwrap();
    let (ctx, _, step, processes) = fixture(d.path(), "", "2.0");
    let exec = ExecutionContext::new(ctx, "tg-op".into(), &SilentSink).with_processes(processes);
    let stage = stage_telegram(&step, &exec).unwrap();
    let moved = d.path().join("moved-installation");
    fs::rename(&stage.root, &moved).unwrap();
    symlink(&moved, &stage.root).unwrap();
    let before = binary_hash(&moved.join("Telegram")).unwrap();
    assert!(activate_telegram(&stage, &exec).is_err());
    assert_eq!(binary_hash(&moved.join("Telegram")).unwrap(), before);
}

struct ChangedRelease(Vec<u8>);
impl HttpTransport for ChangedRelease {
    fn send(&self, _: &str, _: &DownloadPolicy) -> AppResult<HttpResponse> {
        Ok(HttpResponse {
            url: "https://td.telegram.org/linux-x64/td-setup-linux-x64-3.0.tar.xz".into(),
            status: 200,
            bytes: self.0.clone(),
            headers: BTreeMap::new(),
        })
    }
}
#[test]
fn telegram_cannot_silently_receive_a_different_fixed_release() {
    let d = tempfile::tempdir().unwrap();
    let (mut ctx, _, step, processes) = fixture(d.path(), "", "2.0");
    ctx.http = HttpClient::with_transport(Arc::new(ChangedRelease(
        fs::read(d.path().join("telegram.tar.xz")).unwrap(),
    )));
    let exec = ExecutionContext::new(ctx, "tg-op".into(), &SilentSink).with_processes(processes);
    assert_eq!(
        stage_telegram(&step, &exec).unwrap_err().code,
        ErrorCode::UnsafeArtifact
    );
}
