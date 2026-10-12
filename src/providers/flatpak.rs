use super::*;
use crate::io::command::CommandSpec;
pub struct FlatpakProvider;
pub fn flatpak_scope(installation: &FlatpakInstall) -> AppResult<String> {
    match installation {
        FlatpakInstall::User => Ok("--user".into()),
        FlatpakInstall::System { name } if name == "default" => Ok("--system".into()),
        FlatpakInstall::System { name } if valid_part(name) => Ok(format!("--installation={name}")),
        _ => Err(AppError::new(
            ErrorCode::InvalidInput,
            "Instalación Flatpak inválida",
        )),
    }
}
fn valid_part(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.len() < 200
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
pub fn target_ref(target: &Target) -> AppResult<(FlatpakInstall, String)> {
    let Locator::Flatpak {
        installation,
        app_id,
        arch,
        branch,
        ..
    } = &target.locator
    else {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Destino no es Flatpak",
        ));
    };
    if ![app_id, arch, branch].iter().all(|s| valid_part(s)) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Referencia Flatpak inválida",
        ));
    }
    Ok((
        installation.clone(),
        format!("app/{app_id}/{arch}/{branch}"),
    ))
}
pub fn flatpak_query(
    ctx: &ProviderContext,
    installation: &FlatpakInstall,
    command: &str,
    flag: Option<&str>,
    remote: Option<&str>,
    reference: &str,
) -> AppResult<String> {
    let mut args = vec![command.to_string(), flatpak_scope(installation)?];
    if let Some(f) = flag {
        args.push(f.into());
    }
    if let Some(remote) = remote {
        if !valid_part(remote) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Remoto Flatpak inválido",
            ));
        }
        args.push(remote.into());
    }
    args.push(reference.into());
    let result = ctx
        .commands
        .run(&CommandSpec::new("/usr/bin/flatpak").args(args))?;
    if result.code != 0 {
        return Err(AppError::new(
            ErrorCode::NetworkError,
            "No se pudo consultar la referencia Flatpak",
        ));
    }
    Ok(result.stdout.trim().to_string())
}
pub fn valid_commit(commit: &str) -> bool {
    commit.len() == 64 && commit.chars().all(|c| c.is_ascii_hexdigit())
}
impl Provider for FlatpakProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let (installation, reference) = target_ref(target)?;
        let installed = flatpak_query(
            ctx,
            &installation,
            "info",
            Some("--show-commit"),
            None,
            &reference,
        )?;
        let origin = flatpak_query(
            ctx,
            &installation,
            "info",
            Some("--show-origin"),
            None,
            &reference,
        )?;
        let available = flatpak_query(
            ctx,
            &installation,
            "remote-info",
            Some("--show-commit"),
            Some(&origin),
            &reference,
        )?;
        if !valid_commit(&installed) || !valid_commit(&available) {
            return Err(AppError::new(
                ErrorCode::NetworkError,
                "Flatpak no informó un commit válido",
            ));
        }
        let state = if installed == available {
            CheckState::Current
        } else {
            CheckState::UpdateAvailable
        };
        let mut check = check_base(target, ctx, state);
        let version = |raw: String| Version {
            raw,
            kind: VersionKind::FlatpakCommit,
            product: None,
        };
        check.installed = Some(version(installed));
        check.available = Some(version(available));
        check.source = Some(format!("Flatpak: {origin}"));
        if let Ok(runtime) = flatpak_query(
            ctx,
            &installation,
            "info",
            Some("--show-runtime"),
            None,
            &reference,
        ) && !runtime.is_empty()
        {
            check.details.push(format!("Runtime: {runtime}"));
            if let Ok(info) = flatpak_query(
                ctx,
                &installation,
                "info",
                None,
                None,
                &format!("runtime/{runtime}"),
            ) && info
                .lines()
                .any(|l| l.contains("End-of-life") || l.contains("end-of-life"))
            {
                check.details.push(
                    "El runtime está fuera de soporte; el proveedor debe publicar una migración"
                        .into(),
                );
            }
        }
        Ok(check)
    }
}
