mod catalog;
mod desktop;
mod ownership;
use crate::{alpm::PackageFacts, domain::*, io::command::*, paths::Paths};
pub use catalog::local_identity;
pub use desktop::*;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
};
pub fn discover(paths: &Paths, facts: &[PackageFacts], io: &CommandRunner) -> AppResult<Vec<App>> {
    let ownership = ownership::Ownership::new(facts);
    let base = catalog::base_packages();
    let browser = io
        .run(
            &CommandSpec::new("/usr/bin/xdg-settings")
                .args(["get".into(), "default-web-browser".into()]),
        )
        .ok()
        .filter(|r| r.code == 0)
        .and_then(|r| {
            let name = r.stdout.trim().trim_end_matches(".desktop").to_string();
            if facts.iter().any(|p| p.name == name) {
                Some(AppId(format!("repo:{name}")))
            } else {
                None
            }
        });
    let mut seen = HashSet::new();
    let mut result: BTreeMap<AppId, App> = BTreeMap::new();
    for directory in &paths.application_dirs {
        let entries = match fs::read_dir(directory) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let mut entries = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let id = path.file_name().unwrap().to_string_lossy().into_owned();
            // Launcher precedence applies within an installation; a second Flatpak
            // installation with the same exported filename is a separate candidate.
            let scope = if directory.ends_with("flatpak/exports/share/applications") {
                if directory.starts_with(&paths.home) {
                    "flatpak-user"
                } else {
                    "flatpak-system"
                }
            } else {
                "desktop"
            };
            if scope == "flatpak-user" && seen.contains(&format!("desktop:{id}")) {
                continue;
            }
            if !seen.insert(format!("{scope}:{id}")) {
                continue;
            }
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let entry = match parse_desktop(&id, &bytes) {
                Ok(Some(e)) => e,
                _ => continue,
            };
            let words = exec_words(&entry.exec).unwrap_or_default();
            let words = ownership::program_words(&words);
            let raw = words.first().map(String::as_str).unwrap_or("");
            let app_token = ownership::application_token(words);
            let exe = ownership::executable(app_token, &paths.home);
            let original = if app_token.starts_with('/') {
                std::path::PathBuf::from(app_token)
            } else {
                std::path::PathBuf::from("/usr/bin").join(app_token)
            };
            let package = if app_token.is_empty() {
                None
            } else {
                ownership
                    .package(&exe)
                    .or_else(|| ownership.package(&original))
            }
            .or_else(|| ownership.package(&path));
            let web = if raw.ends_with("omarchy-launch-webapp") {
                words
                    .get(1)
                    .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
                    .cloned()
            } else {
                words.iter().find_map(|w| {
                    w.strip_prefix("--app=")
                        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
                        .map(String::from)
                })
            };
            let (app_id, locator, installed, suggested) = if let Some(url) = web {
                let id = AppId(format!("web:{:x}", sha2::Sha256::digest(url.as_bytes())));
                (
                    id,
                    Locator::Web {
                        url,
                        browser: browser.clone(),
                    },
                    None,
                    true,
                )
            } else if raw.ends_with("flatpak") && words.iter().any(|w| w == "run") {
                let app_id = words
                    .iter()
                    .skip(1)
                    .find(|w| !w.starts_with('-') && w.contains('.') && !w.contains('/'))
                    .cloned()
                    .unwrap_or_default();
                let branch = words
                    .iter()
                    .find_map(|w| w.strip_prefix("--branch="))
                    .unwrap_or("stable")
                    .to_string();
                let arch = words
                    .iter()
                    .find_map(|w| w.strip_prefix("--arch="))
                    .unwrap_or("x86_64")
                    .to_string();
                let installation = if path.starts_with(&paths.home) {
                    FlatpakInstall::User
                } else {
                    FlatpakInstall::System {
                        name: words
                            .iter()
                            .find_map(|w| w.strip_prefix("--installation="))
                            .unwrap_or("default")
                            .into(),
                    }
                };
                let scope = if installation == FlatpakInstall::User {
                    "user"
                } else {
                    "system"
                };
                (
                    AppId(format!("flatpak:{scope}:{app_id}/{arch}/{branch}")),
                    Locator::Flatpak {
                        installation,
                        app_id,
                        arch,
                        branch,
                        origin: String::new(),
                    },
                    None,
                    true,
                )
            } else if let Some((id, locator)) = catalog::recognized_local(&exe, paths) {
                (id, locator, None, true)
            } else if let Some(pkg) = package {
                let selected =
                    pkg.explicit && !base.contains(&pkg.name) && pkg.name != "hermes-desktop";
                (
                    AppId(format!("repo:{}", pkg.name)),
                    Locator::Repo {
                        package: pkg.name.clone(),
                    },
                    Some(Version::arch(&pkg.version)),
                    selected,
                )
            } else {
                let id = AppId(catalog::local_identity("unknown", &path));
                (
                    id,
                    Locator::Unknown {
                        path: if raw.is_empty() { None } else { Some(exe) },
                    },
                    None,
                    false,
                )
            };
            let target_id = TargetId(app_id.0.clone());
            if let Some(existing) = result.get_mut(&app_id) {
                existing.desktop_ids.push(id);
                continue;
            }
            let mut components = vec![Target {
                id: target_id,
                locator,
                installed,
            }];
            if matches!(&components[0].locator,Locator::Repo{package}if package=="steam") {
                components.push(Target {
                    id: "steam:client".into(),
                    locator: Locator::SteamClient {
                        root: paths.home.join(".local/share/Steam"),
                        channel: crate::providers::steam::detect_channel(
                            &paths.home.join(".local/share/Steam"),
                        )
                        .unwrap_or_else(|_| "unsupported".into()),
                    },
                    installed: None,
                });
            }
            result.insert(
                app_id.clone(),
                App {
                    id: app_id,
                    name: entry.name,
                    desktop_ids: vec![id],
                    components,
                    suggested,
                },
            );
        }
    }
    let mut apps = result.into_values().collect::<Vec<_>>();
    apps.sort_by_key(|a| a.name.to_lowercase());
    Ok(apps)
}
use sha2::Digest;
