use omarchy_app_updates::{
    domain::*,
    io::{command::*, http::*},
    paths::Paths,
    providers::{
        Provider, ProviderContext,
        releases::{annotate_published, classify, parse_release, published_version},
        steam::SteamProvider,
        telegram::{TelegramProvider, TelegramReceipt},
        web::WebProvider,
    },
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
struct Source {
    body: Vec<u8>,
    status: u16,
    calls: Arc<Mutex<Vec<String>>>,
}
impl HttpTransport for Source {
    fn send(&self, url: &str, _: &DownloadPolicy) -> AppResult<HttpResponse> {
        self.calls.lock().unwrap().push(url.into());
        Ok(HttpResponse {
            url: url.into(),
            status: self.status,
            bytes: self.body.clone(),
            headers: BTreeMap::new(),
        })
    }
}
struct NoCommands;
impl CommandBackend for NoCommands {
    fn run(&self, _: &CommandSpec) -> AppResult<Captured> {
        panic!("providers must not launch desktop apps")
    }
}
fn ctx(home: std::path::PathBuf, body: &[u8], status: u16) -> ProviderContext {
    ProviderContext {
        paths: Paths::for_home(home),
        commands: CommandRunner::with_backend(Arc::new(NoCommands)),
        http: HttpClient::with_transport(Arc::new(Source {
            body: body.into(),
            status,
            calls: Arc::new(Mutex::new(vec![])),
        })),
        now: 100,
    }
}
fn repo(name: &str, version: &str) -> Target {
    Target {
        id: name.into(),
        locator: Locator::Repo {
            package: name.into(),
        },
        installed: Some(Version::arch(version)),
    }
}
#[test]
fn publisher_and_install_channel_are_distinct() {
    assert_eq!(
        classify(
            &Version::arch("1:10.0-2"),
            Some(&Version::arch("1:10.0-2")),
            Some(&Version::product("11.0"))
        )
        .unwrap(),
        CheckState::PublisherAhead
    );
    assert_eq!(
        classify(
            &Version::arch("12.0-1"),
            Some(&Version::arch("11.0-3")),
            Some(&Version::product("11.0"))
        )
        .unwrap(),
        CheckState::InstalledAhead
    );
    assert_eq!(
        classify(
            &Version::arch("10.0-1"),
            Some(&Version::arch("10.0-2")),
            Some(&Version::product("11.0"))
        )
        .unwrap(),
        CheckState::UpdateAvailable
    );
    assert_eq!(
        parse_release(
            "github_release",
            &serde_json::json!({"tag_name":"v12.0.0-rc1","prerelease":true,"draft":false})
        )
        .unwrap(),
        None
    );
    assert_eq!(
        parse_release(
            "github_tags",
            &serde_json::json!([{"name":"tip"},{"name":"v1.4.0-rc1"},{"name":"v1.3.1"}])
        )
        .unwrap(),
        Some(Version::product("1.3.1"))
    );
}
#[test]
fn publication_failures_preserve_verified_channel_and_have_details() {
    let d = tempfile::tempdir().unwrap();
    let t = repo("google-chrome", "155.0.8059.39-2");
    for (body, status) in [(b"{altered".as_slice(), 200), (b"{}".as_slice(), 429)] {
        let c = ctx(d.path().into(), body, status);
        let mut check = omarchy_app_updates::providers::check_base(&t, &c, CheckState::Current);
        check.available = t.installed.clone();
        annotate_published(&mut check, &t, &c);
        assert_eq!(check.state, CheckState::Current);
        assert!(check.published.is_none());
        assert!(check.details.iter().any(|s| s.contains("publicación")));
    }
}
#[test]
fn official_sources_parse_public_versions_and_do_not_invent_ticktick() {
    let d = tempfile::tempdir().unwrap();
    for (name, body, expected) in [
        (
            "google-chrome",
            include_bytes!("fixtures/releases/chrome.json").as_slice(),
            "155.0.8059.39",
        ),
        (
            "thunderbird",
            include_bytes!("fixtures/releases/thunderbird.json").as_slice(),
            "157.0.1",
        ),
        (
            "visual-studio-code-bin",
            include_bytes!("fixtures/releases/vscode.json").as_slice(),
            "1.141.0",
        ),
        (
            "audacity",
            include_bytes!("fixtures/releases/audacity.json").as_slice(),
            "4.0.1",
        ),
        (
            "remmina",
            include_bytes!("fixtures/releases/remmina.json").as_slice(),
            "1.4.43",
        ),
        (
            "voxtype-bin",
            include_bytes!("fixtures/releases/voxtype.json").as_slice(),
            "1.1.0",
        ),
    ] {
        assert_eq!(
            published_version(&repo(name, "1.0-1"), &ctx(d.path().into(), body, 200))
                .unwrap()
                .unwrap()
                .raw,
            expected
        );
    }
    assert!(
        published_version(
            &repo("ticktick", "8.0.20-1"),
            &ctx(d.path().into(), b"{}", 200)
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn steam_client_manifest_is_separate_from_launcher_package() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("Steam");
    std::fs::create_dir_all(root.join("package")).unwrap();
    std::fs::write(
        root.join("package/steam_client_ubuntu12.manifest"),
        b"\"ubuntu12\" { \"version\" \"1788652215\" }",
    )
    .unwrap();
    let t = Target {
        id: "steam:client".into(),
        locator: Locator::SteamClient {
            root,
            channel: "ubuntu12".into(),
        },
        installed: None,
    };
    let c = SteamProvider
        .check(
            &t,
            &ctx(
                d.path().into(),
                b"\"ubuntu12\" { \"version\" \"1788652216\" }",
                200,
            ),
        )
        .unwrap();
    assert_eq!(c.installed.unwrap().raw, "1788652215");
    assert_eq!(c.state, CheckState::UpdateAvailable);
    assert_ne!(t.id, repo("steam", "1.0.0.87-3").id);
}
#[test]
fn telegram_missing_or_changed_binary_receipt_stays_unknown() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("Telegram");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("Telegram"), b"binary").unwrap();
    let t = Target {
        id: "telegram:local".into(),
        locator: Locator::TelegramLocal { root: root.clone() },
        installed: None,
    };
    let c = ctx(
        d.path().into(),
        include_bytes!("fixtures/releases/telegram.json"),
        200,
    );
    assert_eq!(
        TelegramProvider.check(&t, &c).unwrap().state,
        CheckState::Unknown
    );
    let receipt = TelegramReceipt {
        version: "7.3.1".into(),
        binary_sha256: "wrong".into(),
    };
    std::fs::write(
        root.join(".app-updates-version.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    assert_eq!(
        TelegramProvider.check(&t, &c).unwrap().state,
        CheckState::Unknown
    );
}
#[test]
fn web_has_no_separate_version_and_never_launches_browser() {
    let d = tempfile::tempdir().unwrap();
    let c = ctx(d.path().into(), b"", 200);
    for browser in [None, Some("repo:google-chrome".into())] {
        let t = Target {
            id: "web:whatsapp".into(),
            locator: Locator::Web {
                url: "https://web.whatsapp.com".into(),
                browser,
            },
            installed: None,
        };
        let check = WebProvider.check(&t, &c).unwrap();
        assert_eq!(check.state, CheckState::WebAutomatic);
        assert!(check.installed.is_none());
        assert!(check.available.is_none());
        assert!(!check.details.is_empty());
    }
}

#[test]
fn steam_beta_uses_its_own_publisher_but_the_real_local_manifest_name() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("Steam");
    std::fs::create_dir_all(root.join("package")).unwrap();
    std::fs::write(
        root.join("package/steam_client_ubuntu12.manifest"),
        b"\"ubuntu12\" { \"version\" \"1791686101\" }",
    )
    .unwrap();
    let target = Target {
        id: "steam:beta".into(),
        locator: Locator::SteamClient {
            root,
            channel: "ubuntu12_beta".into(),
        },
        installed: None,
    };
    assert_eq!(
        omarchy_app_updates::providers::steam::manifest_url("ubuntu12_beta").unwrap(),
        "https://client-update.akamai.steamstatic.com/steam_client_publicbeta_ubuntu12"
    );
    assert_eq!(
        SteamProvider
            .check(
                &target,
                &ctx(
                    d.path().into(),
                    b"\"ubuntu12\" { \"version\" \"1791686101\" }",
                    200
                )
            )
            .unwrap()
            .state,
        CheckState::Current
    );
}
#[test]
fn telegram_verified_binary_metadata_confirms_current() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("Telegram");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("Telegram"), b"binary").unwrap();
    let receipt = TelegramReceipt {
        version: "7.3.1".into(),
        binary_sha256: omarchy_app_updates::providers::telegram::binary_hash(
            &root.join("Telegram"),
        )
        .unwrap(),
    };
    std::fs::write(
        root.join(".app-updates-version.json"),
        serde_json::to_vec(&receipt).unwrap(),
    )
    .unwrap();
    let t = Target {
        id: "telegram".into(),
        locator: Locator::TelegramLocal { root },
        installed: None,
    };
    assert_eq!(
        TelegramProvider
            .check(
                &t,
                &ctx(
                    d.path().into(),
                    include_bytes!("fixtures/releases/telegram.json"),
                    200
                )
            )
            .unwrap()
            .state,
        CheckState::Current
    );
}
#[test]
#[ignore = "host read-only official source acceptance"]
fn live_official_sources_and_steam_manifest_parse() {
    let context = ProviderContext::new(Paths::for_home("/home/korvo".into()));
    for name in [
        "google-chrome",
        "thunderbird",
        "visual-studio-code-bin",
        "ghostty",
        "audacity",
        "protege",
        "voxtype-bin",
        "remmina",
    ] {
        let version = published_version(&repo(name, "1.0-1"), &context)
            .unwrap()
            .unwrap();
        println!("{name}: {}", version.raw);
    }
    let steam = Target {
        id: "steam:client".into(),
        locator: Locator::SteamClient {
            root: "/home/korvo/.local/share/Steam".into(),
            channel: "ubuntu12".into(),
        },
        installed: None,
    };
    let check = SteamProvider.check(&steam, &context).unwrap();
    println!("Steam client: {:?}", check.state);
}
