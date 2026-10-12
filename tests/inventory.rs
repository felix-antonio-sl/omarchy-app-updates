use omarchy_app_updates::{
    alpm::PackageFacts,
    domain::Locator,
    inventory::{discover, parse_desktop},
    io::command::CommandRunner,
    paths::Paths,
};
use std::{fs, os::unix::fs::PermissionsExt};
fn fact(name: &str, exe: &str) -> PackageFacts {
    PackageFacts {
        name: name.into(),
        version: "1.0-1".into(),
        explicit: true,
        files: vec![exe.trim_start_matches('/').into()],
        url: "https://example.org".into(),
        depends: vec![],
        provides: vec![],
        groups: vec![],
        ignored: false,
    }
}
#[test]
fn desktop_fields_are_data() {
    let entry=parse_desktop("demo.desktop",b"[Desktop Entry]\nType=Application\nName=Clinica <b>LE</b>\nExec=\"/tmp/path with spaces/app\" %U\nVersion=1.0\n").unwrap().unwrap();
    assert_eq!(entry.name, "Clinica <b>LE</b>");
    assert!(entry.exec.contains("%U"));
    assert!(
        parse_desktop("hidden.desktop", b"[Desktop Entry]\nHidden=true\n")
            .unwrap()
            .is_none()
    );
    assert!(
        parse_desktop(
            "hidden.desktop",
            b"[Desktop Entry]\nNoDisplay=true\nName=Hidden\nExec=hidden\n"
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn precedence_and_package_grouping() {
    let d = tempfile::tempdir().unwrap();
    let user = d.path().join("user");
    let system = d.path().join("system");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&system).unwrap();
    fs::write(
        system.join("ghostty.desktop"),
        "[Desktop Entry]\nName=Ghostty\nExec=/usr/bin/ghostty\n",
    )
    .unwrap();
    fs::write(
        user.join("ghostty.desktop"),
        "[Desktop Entry]\nHidden=true\n",
    )
    .unwrap();
    for n in ["calc", "writer"] {
        fs::write(
            system.join(format!("{n}.desktop")),
            format!("[Desktop Entry]\nName=LibreOffice {n}\nExec=/usr/bin/libreoffice --{n}\n"),
        )
        .unwrap();
    }
    let mut paths = Paths::for_home(d.path().into());
    paths.application_dirs = vec![user, system];
    let apps = discover(
        &paths,
        &[
            fact("ghostty", "/usr/bin/ghostty"),
            fact("libreoffice-fresh", "/usr/bin/libreoffice"),
        ],
        &CommandRunner::default(),
    )
    .unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].id.0, "repo:libreoffice-fresh");
    assert_eq!(apps[0].desktop_ids.len(), 2);
}
#[test]
fn untrusted_launchers_are_not_executed_and_web_has_no_app_version() {
    let d = tempfile::tempdir().unwrap();
    let appsdir = d.path().join("apps");
    fs::create_dir_all(&appsdir).unwrap();
    let marker = d.path().join("executed");
    let executable = d.path().join("marker app");
    fs::write(
        &executable,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        appsdir.join("marker.desktop"),
        format!(
            "[Desktop Entry]\nName=Clínica <b>LE</b>\nExec=\"{}\" %U\n",
            executable.display()
        ),
    )
    .unwrap();
    fs::write(appsdir.join("WhatsApp.desktop"),"[Desktop Entry]\nName=WhatsApp\nVersion=1.0\nExec=omarchy-launch-webapp https://web.whatsapp.com/\n").unwrap();
    let mut paths = Paths::for_home(d.path().into());
    paths.application_dirs = vec![appsdir];
    let apps = discover(&paths, &[], &CommandRunner::default()).unwrap();
    assert!(!marker.exists());
    assert!(apps.iter().any(|a| a.name == "Clínica <b>LE</b>"));
    let web = apps.iter().find(|a| a.name == "WhatsApp").unwrap();
    assert!(matches!(web.components[0].locator, Locator::Web { .. }));
    assert!(web.components[0].installed.is_none());
}
#[test]
fn separate_local_installations_remain_separate() {
    let d = tempfile::tempdir().unwrap();
    let appsdir = d.path().join("apps");
    fs::create_dir_all(&appsdir).unwrap();
    for n in ["one", "two"] {
        let root = d.path().join(n);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Telegram"), b"public binary fixture").unwrap();
        fs::write(
            appsdir.join(format!("{n}.desktop")),
            format!(
                "[Desktop Entry]\nName=Telegram\nExec={}\n",
                root.join("Telegram").display()
            ),
        )
        .unwrap();
    }
    let mut paths = Paths::for_home(d.path().into());
    paths.application_dirs = vec![appsdir];
    let apps = discover(&paths, &[], &CommandRunner::default()).unwrap();
    assert_eq!(apps.len(), 2);
    assert_ne!(apps[0].id, apps[1].id);
}

#[test]
fn interpreter_is_not_the_application_owner() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().join("apps");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("myapp.desktop"),
        "[Desktop Entry]\nName=My app\nExec=/usr/bin/python /opt/myapp/app.py\n",
    )
    .unwrap();
    let mut paths = Paths::for_home(d.path().into());
    paths.application_dirs = vec![dir];
    let apps = discover(
        &paths,
        &[
            fact("python", "/usr/bin/python"),
            fact("myapp", "/opt/myapp/app.py"),
        ],
        &CommandRunner::default(),
    )
    .unwrap();
    assert_eq!(apps[0].id.0, "repo:myapp");
}
