use omarchy_app_updates::{
    alpm::{AvailablePackage, RepoConfig},
    domain::*,
    io::{
        command::CommandRunner,
        http::{DownloadPolicy, HttpClient, HttpResponse, HttpTransport},
    },
    paths::Paths,
    providers::{
        Provider, ProviderContext,
        arch::{ArchProvider, RepoSnapshot, checkupdates_success},
        aur::AurProvider,
    },
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Rpc {
    calls: Arc<AtomicUsize>,
}
impl HttpTransport for Rpc {
    fn send(&self, _: &str, _: &DownloadPolicy) -> AppResult<HttpResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(HttpResponse{url:"https://aur.archlinux.org/rpc/v5/info".into(),status:200,headers:BTreeMap::new(),bytes:br#"{"version":5,"type":"multiinfo","resultcount":1,"results":[{"ID":123,"Name":"ghostty","PackageBase":"ghostty","Version":"1.3.2-1","URL":"https://ghostty.org","Maintainer":"maintainer","OutOfDate":null}]}"#.to_vec()})
    }
}
fn target(kind: Locator) -> Target {
    Target {
        id: "repo:ghostty".into(),
        locator: kind,
        installed: Some(Version::arch("1.3.1-2")),
    }
}
fn pkg(repo: &str, version: &str) -> AvailablePackage {
    AvailablePackage {
        name: "ghostty".into(),
        version: version.into(),
        repository: repo.into(),
        download_bytes: 100,
        filename: "ghostty.pkg.tar.zst".into(),
        sha256: "a".repeat(64),
        depends: vec![],
        provides: vec![],
        files: vec![],
    }
}
#[test]
fn configured_priority_wins_and_repo_does_not_query_aur() {
    let d = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let ctx = ProviderContext {
        paths: Paths::for_home(d.path().into()),
        commands: CommandRunner::default(),
        http: HttpClient::with_transport(Arc::new(Rpc {
            calls: count.clone(),
        })),
        now: 100,
    };
    let snapshot = RepoSnapshot {
        installed: vec![],
        config_digest: "fixture-config".into(),
        repositories: vec![
            RepoConfig {
                name: "extra".into(),
                servers: vec![],
                siglevel: vec![],
                usage: vec![],
            },
            RepoConfig {
                name: "omarchy".into(),
                servers: vec![],
                siglevel: vec![],
                usage: vec![],
            },
        ],
        packages: vec![pkg("omarchy", "1.3.1-3"), pkg("extra", "1.3.1-2")],
        fingerprint: "fixture".into(),
        dbpath: d.path().into(),
        created_at: 100,
    };
    let check = ArchProvider::new(Ok(snapshot))
        .check(
            &target(Locator::Repo {
                package: "ghostty".into(),
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(check.state, CheckState::Current);
    assert_eq!(check.available.unwrap().raw, "1.3.1-2");
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
#[test]
fn mirror_failure_is_not_current_and_exit_two_is_successful() {
    assert!(checkupdates_success(2).is_ok());
    assert!(checkupdates_success(0).is_ok());
    assert!(checkupdates_success(1).is_err());
    let d = tempfile::tempdir().unwrap();
    let ctx = ProviderContext {
        paths: Paths::for_home(d.path().into()),
        commands: CommandRunner::default(),
        http: HttpClient::default(),
        now: 100,
    };
    assert!(
        ArchProvider::new(Err(AppError::new(ErrorCode::NetworkError, "mirror failed")))
            .check(
                &target(Locator::Repo {
                    package: "ghostty".into()
                }),
                &ctx
            )
            .is_err()
    );
}
#[test]
fn aur_metadata_is_data_and_repo_disappearance_requires_review() {
    let d = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let ctx = ProviderContext {
        paths: Paths::for_home(d.path().into()),
        commands: CommandRunner::default(),
        http: HttpClient::with_transport(Arc::new(Rpc { calls: count })),
        now: 100,
    };
    let check = AurProvider
        .check(
            &target(Locator::Aur {
                package: "ghostty".into(),
                base: "ghostty".into(),
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(check.state, CheckState::UpdateAvailable);
    assert_eq!(check.available.unwrap().raw, "1.3.2-1");
    let snapshot = RepoSnapshot {
        installed: vec![],
        config_digest: "fixture-config".into(),
        repositories: vec![],
        packages: vec![],
        fingerprint: "empty".into(),
        dbpath: d.path().into(),
        created_at: 100,
    };
    let check = ArchProvider::new(Ok(snapshot))
        .check(
            &target(Locator::Repo {
                package: "ghostty".into(),
            }),
            &ctx,
        )
        .unwrap();
    assert_eq!(check.state, CheckState::Unknown);
    assert!(check.details.iter().any(|s| s.contains("origen")));
    assert!(!d.path().join("PKGBUILD").exists());
}
#[test]
fn ignored_package_remains_visible_as_a_restriction() {
    let d = tempfile::tempdir().unwrap();
    let ctx = ProviderContext {
        paths: Paths::for_home(d.path().into()),
        commands: CommandRunner::default(),
        http: HttpClient::default(),
        now: 100,
    };
    let snapshot = RepoSnapshot {
        installed: vec![],
        config_digest: "fixture-config".into(),
        repositories: vec![RepoConfig {
            name: "extra".into(),
            servers: vec![],
            siglevel: vec![],
            usage: vec![],
        }],
        packages: vec![pkg("extra", "1.3.2-1")],
        fingerprint: "x".into(),
        dbpath: d.path().into(),
        created_at: 100,
    };
    let check = ArchProvider::new(Ok(snapshot))
        .with_ignored(vec!["ghost*".into()])
        .check(
            &target(Locator::Repo {
                package: "ghostty".into(),
            }),
            &ctx,
        )
        .unwrap();
    assert!(check.details.iter().any(|d| d.contains("IgnorePkg")));
}

#[test]
#[ignore = "explicit host-only read check; uses configured repository network"]
fn live_snapshot_leaves_system_databases_unchanged() {
    use sha2::{Digest, Sha256};
    use std::fs;
    fn signature() -> Vec<(String, String, u128)> {
        let mut result = vec![];
        for entry in fs::read_dir("/var/lib/pacman/sync").unwrap() {
            let p = entry.unwrap().path();
            if !p.is_file() {
                continue;
            }
            let hash = format!("{:x}", Sha256::digest(fs::read(&p).unwrap()));
            let time = p
                .metadata()
                .unwrap()
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            result.push((
                p.file_name().unwrap().to_string_lossy().into_owned(),
                hash,
                time,
            ));
        }
        result.sort();
        result
    }
    let before = signature();
    let ctx = ProviderContext::new(Paths::for_home("/home/korvo".into()));
    let snapshot = omarchy_app_updates::providers::arch::sync_snapshot(&ctx).unwrap();
    assert!(snapshot.packages.len() > 1000);
    assert_eq!(before, signature());
    println!(
        "readonly snapshot: {} packages, system databases unchanged",
        snapshot.packages.len()
    );
}
