use crate::{domain::*, paths::Paths};
use sha2::{Digest, Sha256};
use std::path::Path;
pub fn local_identity(kind: &str, path: &Path) -> String {
    format!(
        "local:{kind}:{:x}",
        Sha256::digest(path.to_string_lossy().as_bytes())
    )
}
pub fn recognized_local(exe: &Path, paths: &Paths) -> Option<(AppId, Locator)> {
    if exe.starts_with(&paths.home) && exe.file_name().is_some_and(|n| n == "Telegram") {
        let root = exe.parent()?.to_path_buf();
        let id = local_identity("telegram", &root);
        return Some((id.into(), Locator::TelegramLocal { root }));
    }
    None
}
pub fn base_packages() -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for file in [
        "/usr/share/omarchy/install/omarchy-base.packages",
        "/usr/share/omarchy/install/omarchy-other.packages",
    ] {
        if let Ok(text) = std::fs::read_to_string(file) {
            for line in text.lines() {
                let word = line.split('#').next().unwrap_or("").trim();
                if !word.is_empty() {
                    set.insert(word.into());
                }
            }
        }
    }
    set
}
