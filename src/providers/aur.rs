use super::*;
use crate::io::http::DownloadPolicy;
pub struct AurProvider;
impl Provider for AurProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let package = match &target.locator {
            Locator::Aur { package, .. } => package,
            _ => return Err(AppError::new(ErrorCode::InvalidInput, "Destino no es AUR")),
        };
        if package.is_empty()
            || !package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Nombre de paquete inválido",
            ));
        }
        let mut url = reqwest::Url::parse("https://aur.archlinux.org/rpc/v5/info").unwrap();
        url.query_pairs_mut().append_pair("arg[]", package);
        let data = ctx.http.json(
            url.as_str(),
            &DownloadPolicy::for_hosts(&["aur.archlinux.org"], 2_097_152),
        )?;
        let row = data
            .get("results")
            .and_then(|r| r.as_array())
            .and_then(|r| r.iter().find(|r| r["Name"] == package.as_str()))
            .ok_or_else(|| AppError::new(ErrorCode::Unsupported, "El paquete no aparece en AUR"))?;
        let version = row["Version"]
            .as_str()
            .ok_or_else(|| AppError::new(ErrorCode::NetworkError, "AUR no informó una versión"))?;
        let available = Version::arch(version);
        let mut check = check_base(
            target,
            ctx,
            version_state(target.installed.as_ref(), &available)?,
        );
        check.available = Some(available);
        check.source = Some(format!("https://aur.archlinux.org/packages/{package}"));
        if !row["OutOfDate"].is_null() {
            check
                .details
                .push("La receta AUR está marcada como desactualizada".into());
        }
        Ok(check)
    }
}
