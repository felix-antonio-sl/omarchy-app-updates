#![allow(dead_code)]
use omarchy_app_updates::{
    alpm::{AvailablePackage, PackageFacts, RepoConfig},
    domain::*,
    paths::Paths,
    policy::Policy,
    providers::arch::RepoSnapshot,
};
pub fn fixture(home: &std::path::Path) -> (Report, RepoSnapshot, Policy, Paths) {
    let paths = Paths::for_home(home.into());
    let source = home.join(".local/share/ai-updaters/updater.py");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source,"TARGETS = ('codex', 'claude', 'antigravity', 'opencode', 'hermes', 'claude-desktop', 'chatgpt')\n").unwrap();
    let policy = Policy::for_paths(&paths).unwrap();
    let target = Target {
        id: "repo:ghostty".into(),
        locator: Locator::Repo {
            package: "ghostty".into(),
        },
        installed: Some(Version::arch("1.0-1")),
    };
    let report = Report {
        apps: vec![App {
            id: "repo:ghostty".into(),
            name: "Ghostty".into(),
            desktop_ids: vec![],
            components: vec![target.clone()],
            suggested: true,
        }],
        checks: vec![Check {
            target_id: target.id,
            installed: target.installed,
            available: Some(Version::arch("2.0-1")),
            published: None,
            checked_at: 100,
            state: CheckState::UpdateAvailable,
            stale: false,
            source: Some("extra".into()),
            details: vec![],
        }],
        selected: vec!["repo:ghostty".into()],
        coverage: Coverage::Verified,
        operation: None,
        checked_at: Some(100),
    };
    let snapshot = RepoSnapshot {
        repositories: vec![RepoConfig {
            name: "extra".into(),
            servers: vec!["https://example.org".into()],
            siglevel: vec!["Required".into()],
            usage: vec![],
        }],
        packages: vec![AvailablePackage {
            name: "ghostty".into(),
            version: "2.0-1".into(),
            repository: "extra".into(),
            filename: "ghostty.pkg.tar.zst".into(),
            download_bytes: 1,
            sha256: "a".repeat(64),
            depends: vec![],
            provides: vec![],
            files: vec![],
        }],
        installed: vec![PackageFacts {
            name: "ghostty".into(),
            version: "1.0-1".into(),
            explicit: true,
            files: vec!["usr/bin/ghostty".into()],
            url: String::new(),
            depends: vec![],
            provides: vec![],
            groups: vec![],
            ignored: false,
        }],
        config_digest: "config".into(),
        fingerprint: "snapshot".into(),
        dbpath: home.into(),
        created_at: 100,
    };
    (report, snapshot, policy, paths)
}
pub fn plan(home: &std::path::Path) -> (UpdatePlan, Policy, Paths) {
    let (r, s, p, paths) = fixture(home);
    (
        omarchy_app_updates::planner::build_plan(&r, &r.selected, &s, &p, 100).unwrap(),
        p,
        paths,
    )
}
