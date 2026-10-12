use super::{AppId, TargetId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: AppId,
    pub name: String,
    pub desktop_ids: Vec<String>,
    pub components: Vec<Target>,
    pub suggested: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: TargetId,
    pub locator: Locator,
    pub installed: Option<Version>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlatpakInstall {
    User,
    System { name: String },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Locator {
    Repo {
        package: String,
    },
    Aur {
        package: String,
        base: String,
    },
    Flatpak {
        installation: FlatpakInstall,
        app_id: String,
        arch: String,
        branch: String,
        origin: String,
    },
    TelegramLocal {
        root: PathBuf,
    },
    SteamClient {
        root: PathBuf,
        channel: String,
    },
    Web {
        url: String,
        browser: Option<AppId>,
    },
    Unknown {
        path: Option<PathBuf>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Version {
    pub raw: String,
    pub kind: VersionKind,
    pub product: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Arch,
    Product,
    FlatpakCommit,
}
impl Version {
    pub fn arch(raw: impl Into<String>) -> Self {
        Self {
            raw: raw.into(),
            kind: VersionKind::Arch,
            product: None,
        }
    }
    pub fn product(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        Self {
            product: Some(raw.clone()),
            raw,
            kind: VersionKind::Product,
        }
    }
}
