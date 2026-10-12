use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramReceipt {
    pub version: String,
    pub binary_sha256: String,
}
pub fn binary_hash(path: &Path) -> AppResult<String> {
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > 1024 * 1024 * 1024 {
        return Err(AppError::new(
            ErrorCode::UnsafeArtifact,
            "Binario local inválido",
        ));
    }
    let mut reader = std::io::BufReader::new(file);
    let mut sha = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        sha.update(&buffer[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}
pub fn installed_version(root: &Path) -> AppResult<Option<Version>> {
    let path = root.join(".app-updates-version.json");
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return Ok(None);
    };
    if !metadata.is_file() || metadata.len() > 4096 {
        return Ok(None);
    }
    let receipt: TelegramReceipt = match serde_json::from_slice(&std::fs::read(path)?) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    if receipt.binary_sha256 != binary_hash(&root.join("Telegram"))? {
        return Ok(None);
    }
    if receipt.version.is_empty()
        || !receipt
            .version
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Ok(None);
    }
    Ok(Some(Version::product(receipt.version)))
}
pub struct TelegramProvider;
impl Provider for TelegramProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let Locator::TelegramLocal { root } = &target.locator else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Destino no es Telegram local",
            ));
        };
        let mut check = check_base(target, ctx, CheckState::Unknown);
        check.installed = installed_version(root)?;
        let published = super::releases::published_version(target, ctx)?;
        check.source = Some("https://telegram.org/dl/desktop/linux".into());
        check.available = published.clone();
        check.published = published;
        if let Some(installed) = &check.installed {
            check.state = super::releases::classify(
                installed,
                check.available.as_ref(),
                check.published.as_ref(),
            )?;
        } else {
            check.details.push(
                "No hay metadatos de versión asociados al binario actual; no se estima una versión"
                    .into(),
            );
        }
        Ok(check)
    }
}
