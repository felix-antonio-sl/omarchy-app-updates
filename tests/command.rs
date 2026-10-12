use omarchy_app_updates::io::command::*;
use std::time::Duration;
#[test]
fn timeout_stops_the_whole_command_group() {
    let d = tempfile::tempdir().unwrap();
    let marker = d.path().join("orphan-output");
    let mut spec = CommandSpec::new("/usr/bin/sh").args([
        "-c".into(),
        format!("(sleep 0.3; printf orphan > '{}') & wait", marker.display()),
    ]);
    spec.timeout = Duration::from_millis(60);
    assert!(CommandRunner::default().run(&spec).is_err());
    std::thread::sleep(Duration::from_millis(450));
    assert!(!marker.exists(), "A timed-out command left a running child");
}
#[test]
fn interpreter_library_override_is_not_inherited() {
    if std::env::var("APP_UPDATES_TEST_CHILD").as_deref() == Ok("yes") {
        let spec = CommandSpec::new("/usr/bin/sh")
            .args(["-c".into(), "printf '%s' \"${LIBRARY:-safe}\"".into()]);
        assert_eq!(CommandRunner::default().run(&spec).unwrap().stdout, "safe");
        return;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "interpreter_library_override_is_not_inherited",
            "--nocapture",
        ])
        .env("LIBRARY", "untrusted-library")
        .env("APP_UPDATES_TEST_CHILD", "yes")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}
