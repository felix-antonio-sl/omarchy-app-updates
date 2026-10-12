use super::*;
pub struct WebProvider;
impl Provider for WebProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let Locator::Web { browser, .. } = &target.locator else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Destino no es una aplicación web",
            ));
        };
        let mut check = check_base(target, ctx, CheckState::WebAutomatic);
        check.installed = None;
        check.details.push("El servicio web se actualiza automáticamente; su navegador se gestiona como aplicación".into());
        if let Some(browser) = browser {
            check.source = Some(format!("Navegador: {}", browser.0));
        } else {
            check
                .details
                .push("El navegador actual no está identificado".into());
        }
        Ok(check)
    }
}
