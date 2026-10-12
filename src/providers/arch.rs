use super::*;
use crate::{
    alpm::{AlpmConfig, AlpmReader, AvailablePackage, PackageFacts, RepoConfig},
    io::command::CommandSpec,
    store::private_dir,
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoSnapshot {
    pub installed: Vec<PackageFacts>,
    pub config_digest: String,
    pub repositories: Vec<RepoConfig>,
    pub packages: Vec<AvailablePackage>,
    pub fingerprint: String,
    pub dbpath: PathBuf,
    pub created_at: u64,
}
impl RepoSnapshot {
    pub fn preferred(&self, name: &str) -> Option<&AvailablePackage> {
        self.repositories.iter().find_map(|r| {
            self.packages
                .iter()
                .find(|p| p.name == name && p.repository == r.name)
        })
    }
}
pub fn checkupdates_success(code: i32) -> AppResult<()> {
    if matches!(code, 0 | 2) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::NetworkError,
            "No se pudieron comprobar los repositorios",
        ))
    }
}
pub fn sync_snapshot(ctx: &ProviderContext) -> AppResult<RepoSnapshot> {
    let config = AlpmConfig::load(&ctx.paths.pacman_conf, &ctx.commands)?;
    let dbpath = ctx
        .paths
        .state
        .join("snapshots")
        .join(uuid::Uuid::new_v4().to_string());
    private_dir(&dbpath)?;
    let mut spec = CommandSpec::new("/usr/bin/checkupdates").args(["--nocolor".into()]);
    spec.env.insert(
        "CHECKUPDATES_DB".into(),
        dbpath.to_string_lossy().into_owned(),
    );
    let result = ctx.commands.run(&spec)?;
    checkupdates_success(result.code)?;
    let reader = AlpmReader::open(&config, &dbpath)?;
    let packages = reader.available(&dbpath.join("sync"))?;
    if packages.is_empty() {
        return Err(AppError::new(
            ErrorCode::NetworkError,
            "La base temporal de repositorios está vacía",
        ));
    }
    let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&packages)?));
    let installed = reader.installed()?;
    let config_digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&config)?));
    Ok(RepoSnapshot {
        installed,
        config_digest,
        repositories: config.repos,
        packages,
        fingerprint,
        dbpath,
        created_at: ctx.now,
    })
}
pub struct ArchProvider {
    snapshot: AppResult<Arc<RepoSnapshot>>,
    ignored: Vec<String>,
}
impl ArchProvider {
    pub fn new(snapshot: AppResult<RepoSnapshot>) -> Self {
        Self {
            snapshot: snapshot.map(Arc::new),
            ignored: vec![],
        }
    }
    pub fn with_ignored(mut self, ignored: Vec<String>) -> Self {
        self.ignored = ignored;
        self
    }
    pub fn snapshot(&self) -> AppResult<&RepoSnapshot> {
        self.snapshot.as_deref().map_err(Clone::clone)
    }
}
impl Provider for ArchProvider {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check> {
        let Locator::Repo { package } = &target.locator else {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Destino no es un paquete de repositorio",
            ));
        };
        let snapshot = self.snapshot()?;
        if let Some(pkg) = snapshot.preferred(package) {
            let available = Version::arch(&pkg.version);
            let mut check = check_base(
                target,
                ctx,
                version_state(target.installed.as_ref(), &available)?,
            );
            check.source = Some(format!("Repositorio {}", pkg.repository));
            check.available = Some(available);
            if self.ignored.iter().any(|p| glob_match(p, package)) {
                check.details.push(
                    "Restricción IgnorePkg: requiere revisar coherencia antes de instalar".into(),
                );
            }
            Ok(check)
        } else {
            let mut check = super::aur::AurProvider.check(
                &Target {
                    locator: Locator::Aur {
                        package: package.clone(),
                        base: package.clone(),
                    },
                    ..target.clone()
                },
                ctx,
            )?;
            check.state = CheckState::Unknown;
            check.details.push("Cambió el origen: el paquete no está en tus repositorios; revisar el proveedor antes de instalar".into());
            Ok(check)
        }
    }
}
pub fn glob_match(pattern: &str, value: &str) -> bool {
    let p = pattern.as_bytes();
    let s = value.as_bytes();
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < s.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            mark = j;
        } else if let Some(k) = star {
            mark += 1;
            j = mark;
            i = k + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}
