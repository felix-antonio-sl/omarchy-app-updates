use omarchy_app_updates::{
    alpm::{AlpmConfig, AlpmReader, arch_compare, validate_abi},
    io::command::CommandRunner,
};
use std::{cmp::Ordering, fs};
#[test]
fn native_versions_and_abi() {
    assert_eq!(
        arch_compare("1:1.4.43-3", "1.4.44-1").unwrap(),
        Ordering::Greater
    );
    assert_eq!(
        arch_compare("1.3.1-2", "1.3.1-1").unwrap(),
        Ordering::Greater
    );
    assert!(validate_abi("16.0.1", "15.0.0").is_err());
    assert!(validate_abi("16.0.1", "16.0.2").is_ok());
    assert!(arch_compare("bad\0input", "1").is_err());
}
#[test]
fn isolated_database_and_includes() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("db");
    fs::create_dir_all(db.join("local/demo-1.0-1")).unwrap();
    fs::write(db.join("local/ALPM_DB_VERSION"), "9\n").unwrap();
    fs::write(db.join("local/demo-1.0-1/desc"),"%NAME%\ndemo\n\n%VERSION%\n1.0-1\n\n%DESC%\nDemo\n\n%ARCH%\nx86_64\n\n%REASON%\n0\n\n%URL%\nhttps://example.org/\n\n%DEPENDS%\nglibc>=2.33\n\n%PROVIDES%\ndemo-abi=1.0\n\n").unwrap();
    fs::write(
        db.join("local/demo-1.0-1/files"),
        "%FILES%\nusr/bin/demo\n\n",
    )
    .unwrap();
    let include = d.path().join("repos.conf");
    fs::write(&include,"[first]\nServer = https://example.org/$repo/os/$arch\n[second]\nServer = https://example.net/$repo/os/$arch\n").unwrap();
    let conf = d.path().join("pacman.conf");
    fs::write(
        &conf,
        format!(
            "[options]\nArchitecture = x86_64\nSigLevel = Never\nInclude = {}\n",
            include.display()
        ),
    )
    .unwrap();
    let cfg = AlpmConfig::load(&conf, &CommandRunner::default()).unwrap();
    assert_eq!(
        cfg.repos
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert_eq!(
        cfg.repos[0].servers[0],
        "https://example.org/first/os/x86_64"
    );
    let reader = AlpmReader::open(&cfg, &db).unwrap();
    let packages = reader.installed().unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "demo");
    assert!(packages[0].explicit);
    assert_eq!(packages[0].depends, vec!["glibc>=2.33"]);
    assert_eq!(
        reader.owner(std::path::Path::new("/usr/bin/demo")).unwrap(),
        Some("demo".into())
    );
    assert!(
        reader
            .owner(std::path::Path::new("/usr/bin/no-app"))
            .unwrap()
            .is_none()
    );
}
