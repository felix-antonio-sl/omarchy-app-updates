use super::*;
use crate::io::http::DownloadPolicy;
pub struct SteamProvider;
pub fn manifest_url(channel: &str) -> AppResult<String> {
    if !matches!(channel, "ubuntu12" | "ubuntu12_beta") {
        return Err(AppError::new(
            ErrorCode::Unsupported,
            "Canal Steam no reconocido",
        ));
    }
    let manifest = if channel == "ubuntu12_beta" {
        "steam_client_publicbeta_ubuntu12"
    } else {
        "steam_client_ubuntu12"
    };
    Ok(format!(
        "https://client-update.akamai.steamstatic.com/{manifest}"
    ))
}
pub fn manifest_version(bytes: &[u8], channel: &str) -> AppResult<Version> {
    manifest_url(channel)?;
    let invalid = || AppError::new(ErrorCode::NetworkError, "Manifiesto Steam inválido");
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid());
    }
    // Valve KeyValues: accept only a single version at the channel's first level.
    let mut tokens = vec![];
    let mut iter = text.chars().peekable();
    while let Some(c) = iter.next() {
        match c {
            '"' => {
                let mut value = String::new();
                let mut closed = false;
                while let Some(c) = iter.next() {
                    if c == '"' {
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        value.push(iter.next().ok_or_else(invalid)?);
                    } else {
                        value.push(c);
                    }
                }
                if !closed {
                    return Err(invalid());
                }
                tokens.push(value);
            }
            '{' | '}' => tokens.push(c.to_string()),
            c if c.is_whitespace() => (),
            _ => return Err(invalid()),
        }
    }
    if tokens.first().map(String::as_str) != Some(channel.trim_end_matches("_beta"))
        || tokens.get(1).map(String::as_str) != Some("{")
    {
        return Err(invalid());
    }
    let mut depth = 0_i32;
    let mut version = None;
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth < 0 {
                    return Err(invalid());
                }
            }
            "version" if depth == 1 => {
                i += 1;
                let v = tokens.get(i).ok_or_else(invalid)?;
                if version.is_some()
                    || v.is_empty()
                    || v.len() > 20
                    || !v.bytes().all(|c| c.is_ascii_digit())
                {
                    return Err(invalid());
                }
                version = Some(Version::product(v));
            }
            _ => (),
        }
        i += 1;
    }
    if depth != 0 {
        return Err(invalid());
    }
    version.ok_or_else(invalid)
}
impl Provider for SteamProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let Locator::SteamClient { root, channel } = &target.locator else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Destino no es el cliente Steam",
            ));
        };
        let url = manifest_url(channel)?;
        let local = root.join("package").join("steam_client_ubuntu12.manifest");
        let installed = manifest_version(&std::fs::read(local)?, channel)?;
        let remote = ctx.http.get(
            &url,
            &DownloadPolicy::for_hosts(&["client-update.akamai.steamstatic.com"], 2 * 1024 * 1024),
        )?;
        let available = manifest_version(&remote.bytes, channel)?;
        let state = super::releases::classify(&installed, Some(&available), Some(&available))?;
        let mut check = check_base(target, ctx, state);
        check.installed = Some(installed);
        check.available = Some(available.clone());
        check.published = Some(available);
        check.source = Some(url);
        check.details.push(
            "Cliente Valve: versión del manifiesto; el paquete lanzador se comprueba por separado"
                .into(),
        );
        Ok(check)
    }
}
