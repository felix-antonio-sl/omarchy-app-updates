use super::*;
use crate::{alpm::arch_compare, io::http::DownloadPolicy};
use serde::Deserialize;
use std::cmp::Ordering;

#[derive(Deserialize)]
pub struct ReleaseSource {
    pub packages: Vec<String>,
    pub url: String,
    pub strategy: String,
    pub website: String,
}
pub fn sources() -> AppResult<Vec<ReleaseSource>> {
    serde_json::from_str(include_str!("../../catalog/release-sources.json")).map_err(Into::into)
}
fn product_version(v: &Version) -> String {
    if let Some(product) = &v.product {
        return product.clone();
    }
    if v.kind == VersionKind::Arch {
        let upstream = v.raw.split_once(':').map_or(v.raw.as_str(), |(_, v)| v);
        upstream
            .rsplit_once('-')
            .map_or(upstream, |(v, _)| v)
            .into()
    } else {
        v.raw.clone()
    }
}
pub fn classify(
    installed: &Version,
    available: Option<&Version>,
    published: Option<&Version>,
) -> AppResult<CheckState> {
    let Some(available) = available else {
        return Ok(CheckState::Unknown);
    };
    if installed.kind == VersionKind::FlatpakCommit || available.kind == VersionKind::FlatpakCommit
    {
        return Ok(if installed.raw == available.raw {
            CheckState::Current
        } else {
            CheckState::UpdateAvailable
        });
    }
    match arch_compare(&installed.raw, &available.raw)? {
        Ordering::Less => return Ok(CheckState::UpdateAvailable),
        Ordering::Greater => return Ok(CheckState::InstalledAhead),
        Ordering::Equal => (),
    }
    if let Some(published) = published {
        match arch_compare(&product_version(installed), &product_version(published))? {
            Ordering::Less => return Ok(CheckState::PublisherAhead),
            Ordering::Greater => return Ok(CheckState::InstalledAhead),
            Ordering::Equal => (),
        }
    }
    Ok(CheckState::Current)
}
fn stable_number(value: &str) -> Option<Version> {
    let value = value.strip_prefix("Audacity-").unwrap_or(value);
    let value = value.strip_prefix('v').unwrap_or(value);
    if value.len() > 100
        || value.split('.').count() < 2
        || value
            .split('.')
            .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(Version::product(value))
}
pub fn parse_release(strategy: &str, json: &serde_json::Value) -> AppResult<Option<Version>> {
    let invalid = || {
        AppError::new(
            ErrorCode::NetworkError,
            "La fuente de publicación cambió su formato",
        )
    };
    let value = match strategy {
        "chrome" => json
            .get("versions")
            .and_then(|v| v.get(0))
            .and_then(|v| v.get("version"))
            .and_then(|v| v.as_str()),
        "thunderbird" => json
            .get("LATEST_THUNDERBIRD_VERSION")
            .and_then(|v| v.as_str()),
        "vscode" => json.get("name").and_then(|v| v.as_str()),
        "github_release" => {
            if json
                .get("draft")
                .and_then(|v| v.as_bool())
                .ok_or_else(invalid)?
                || json
                    .get("prerelease")
                    .and_then(|v| v.as_bool())
                    .ok_or_else(invalid)?
            {
                return Ok(None);
            }
            json.get("tag_name").and_then(|v| v.as_str())
        }
        "gitlab_release" => {
            let mut versions = vec![];
            for release in json.as_array().ok_or_else(invalid)? {
                if release.get("upcoming_release").and_then(|v| v.as_bool()) != Some(false) {
                    continue;
                }
                if let Some(v) = release
                    .get("tag_name")
                    .and_then(|v| v.as_str())
                    .and_then(stable_number)
                {
                    versions.push(v);
                }
            }
            return latest(versions);
        }
        "github_tags" => {
            let versions = json
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .filter_map(|v| {
                    v.get("name")
                        .and_then(|v| v.as_str())
                        .and_then(stable_number)
                })
                .collect();
            return latest(versions);
        }
        _ => return Err(invalid()),
    };
    value.and_then(stable_number).map(Some).ok_or_else(invalid)
}
fn latest(versions: Vec<Version>) -> AppResult<Option<Version>> {
    let mut best: Option<Version> = None;
    for v in versions {
        if best
            .as_ref()
            .is_none_or(|b| arch_compare(&v.raw, &b.raw).is_ok_and(|o| o == Ordering::Greater))
        {
            best = Some(v);
        }
    }
    if best.is_none() {
        return Err(AppError::new(
            ErrorCode::NetworkError,
            "La fuente no informó una publicación estable",
        ));
    }
    Ok(best)
}
pub fn source_for(target: &Target) -> AppResult<Option<ReleaseSource>> {
    let name = match &target.locator {
        Locator::Repo { package } | Locator::Aur { package, .. } => package.as_str(),
        Locator::TelegramLocal { .. } => "telegram-local",
        _ => return Ok(None),
    };
    Ok(sources()?
        .into_iter()
        .find(|s| s.packages.iter().any(|p| p == name)))
}
pub fn published_version(target: &Target, ctx: &ProviderContext) -> AppResult<Option<Version>> {
    let Some(source) = source_for(target)? else {
        return Ok(None);
    };
    let host = reqwest::Url::parse(&source.url)
        .map_err(|_| AppError::new(ErrorCode::InvalidInput, "Fuente inválida"))?
        .host_str()
        .map(str::to_string)
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "Fuente sin host"))?;
    let policy = DownloadPolicy::for_hosts(&[host.as_str()], 2 * 1024 * 1024);
    parse_release(&source.strategy, &ctx.http.json(&source.url, &policy)?)
}
pub fn annotate_published(check: &mut Check, target: &Target, ctx: &ProviderContext) {
    if let Ok(Some(source)) = source_for(target) {
        check
            .details
            .push(format!("Fuente del fabricante: {}", source.website));
    }
    match published_version(target,ctx) {
        Ok(Some(v)) => {
            if let Some(installed) = &check.installed {
                if let Ok(state) = classify(installed,check.available.as_ref(),Some(&v)) {
                    if matches!(check.state,CheckState::Current|CheckState::UpdateAvailable|CheckState::InstalledAhead) { check.state = state; }
                }
            }
            check.published = Some(v);
        },
        Ok(None) => check.details.push("Sin fuente de publicación estable verificable".into()),
        Err(_) => check.details.push("No se pudo comprobar la publicación del fabricante; se conserva el resultado del canal instalado".into()),
    }
}
