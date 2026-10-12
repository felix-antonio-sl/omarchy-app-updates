use omarchy_app_updates::{domain::*, policy::Policy};
use std::{fs, path::Path};
fn app(name: &str, package: &str, suggested: bool) -> App {
    App {
        id: format!("repo:{package}").into(),
        name: name.into(),
        desktop_ids: vec![format!("{package}.desktop")],
        components: vec![Target {
            id: format!("repo:{package}").into(),
            locator: Locator::Repo {
                package: package.into(),
            },
            installed: Some(Version::arch("1.0-1")),
        }],
        suggested,
    }
}
fn source(home: &Path, targets: &str) -> std::path::PathBuf {
    let p = home.join(".local/share/ai-updaters/updater.py");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        &p,
        format!("TARGETS={targets}\nraise Exception('this module must never execute')\n"),
    )
    .unwrap();
    p
}
#[test]
fn mandatory_exclusions_override_saved_selection() {
    let d = tempfile::tempdir().unwrap();
    let src = source(
        d.path(),
        "('codex','claude','antigravity','opencode','hermes','claude-desktop','chatgpt')",
    );
    let policy = Policy::load(&src, Path::new("catalog/managed-exclusions.json")).unwrap();
    assert!(policy.coverage.verified());
    let apps = vec![
        app("Codex CLI", "codex", true),
        app("Claude Code", "claude-code", true),
        app("Antigravity CLI", "antigravity-cli", true),
        app("OpenCode", "opencode", true),
        app("Hermes CLI", "hermes", true),
        app("Claude", "claude-desktop", true),
        app("ChatGPT", "chatgpt", true),
        app("Steam", "steam", true),
        app("Hermes", "hermes-desktop", false),
        app("Base app", "base-app", false),
    ];
    assert_eq!(policy.visible(&apps).len(), 3);
    let config = UserConfig {
        selected: apps.iter().map(|a| a.id.clone()).collect(),
        initialized: true,
        ..Default::default()
    };
    let selected = policy.selected(&apps, &config).unwrap();
    assert_eq!(selected.len(), 3);
    assert!(selected.contains(&"repo:base-app".into()));
    let initial = policy.selected(&apps, &UserConfig::default()).unwrap();
    assert_eq!(initial, vec![AppId::from("repo:steam")]);
    assert!(policy.selected(&[], &config).unwrap().is_empty());
}
#[test]
fn changed_or_nonliteral_coverage_is_not_verified() {
    let d = tempfile::tempdir().unwrap();
    for targets in [
        "('codex','claude','antigravity','opencode','hermes','claude-desktop','chatgpt','newtool')",
        "tuple(run_installer())",
    ] {
        let src = source(d.path(), targets);
        let p = Policy::load(&src, Path::new("catalog/managed-exclusions.json")).unwrap();
        assert!(!p.coverage.verified());
        assert!(p.visible(&[app("ChatGPT", "chatgpt", true)]).is_empty());
    }
}

#[test]
fn reassignment_cannot_hide_an_additional_managed_target() {
    let d = tempfile::tempdir().unwrap();
    let src = source(
        d.path(),
        "('codex','claude','antigravity','opencode','hermes','claude-desktop','chatgpt')",
    );
    fs::write(
        &src,
        format!(
            "{}\nTARGETS += ('newtool',)\n",
            fs::read_to_string(&src).unwrap()
        ),
    )
    .unwrap();
    assert!(
        !Policy::load(&src, Path::new("catalog/managed-exclusions.json"))
            .unwrap()
            .coverage
            .verified()
    );
}
#[test]
fn protected_local_aliases_are_excluded_but_web_sites_are_not() {
    let d = tempfile::tempdir().unwrap();
    let src = source(
        d.path(),
        "('codex','claude','antigravity','opencode','hermes','claude-desktop','chatgpt')",
    );
    let p = Policy::load(&src, Path::new("catalog/managed-exclusions.json")).unwrap();
    let local = App {
        id: "local:agy".into(),
        name: "Custom label".into(),
        desktop_ids: vec!["custom.desktop".into()],
        components: vec![Target {
            id: "local:agy".into(),
            locator: Locator::Unknown {
                path: Some(d.path().join(".local/bin/agy")),
            },
            installed: None,
        }],
        suggested: true,
    };
    assert!(p.visible(&[local]).is_empty());
    let web = App {
        id: "web:chatgpt".into(),
        name: "ChatGPT".into(),
        desktop_ids: vec!["ChatGPT Web.desktop".into()],
        components: vec![Target {
            id: "web:chatgpt".into(),
            locator: Locator::Web {
                url: "https://chatgpt.com/".into(),
                browser: None,
            },
            installed: None,
        }],
        suggested: true,
    };
    assert_eq!(p.visible(&[web]).len(), 1);
}

#[test]
fn cached_reports_and_selection_requests_enforce_policy() {
    use omarchy_app_updates::{
        paths::Paths,
        protocol::{decode_request, dispatch},
        store::Store,
    };
    let d = tempfile::tempdir().unwrap();
    source(
        d.path(),
        "('codex','claude','antigravity','opencode','hermes','claude-desktop','chatgpt')",
    );
    let paths = Paths::for_home(d.path().into());
    let store = Store::new(paths.clone());
    let mut state = store.load().unwrap();
    state.report.apps = vec![app("Codex CLI", "codex", true), app("Steam", "steam", true)];
    state.config.initialized = true;
    state.config.selected = vec!["repo:codex".into(), "repo:steam".into()];
    store.save(&state).unwrap();
    let req = decode_request(
        br#"{"protocol_version":1,"request_id":"status-policy","action":"status","payload":{}}"#,
    )
    .unwrap();
    let resp = dispatch(req, &paths).unwrap();
    assert_eq!(resp.result.unwrap()["apps"].as_array().unwrap().len(), 1);
    let req=decode_request(br#"{"protocol_version":1,"request_id":"select-ok","action":"selection","payload":{"selected":["repo:steam"]}}"#).unwrap();
    assert!(dispatch(req, &paths).unwrap().error.is_none());
    let req=decode_request(br#"{"protocol_version":1,"request_id":"select-denied","action":"selection","payload":{"selected":["repo:codex"]}}"#).unwrap();
    let response = dispatch(req, &paths).unwrap();
    assert!(response.error.is_some());
    assert_eq!(response.request_id, "select-denied");
    assert_eq!(
        store.load().unwrap().config.selected,
        vec![AppId::from("repo:steam")]
    );
}
