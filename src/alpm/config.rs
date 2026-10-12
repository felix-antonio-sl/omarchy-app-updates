use crate::{domain::*, io::command::*};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone)]
pub struct RepoConfig {
    pub name: String,
    pub servers: Vec<String>,
    pub siglevel: Vec<String>,
    pub usage: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct AlpmConfig {
    pub root: PathBuf,
    pub dbpath: PathBuf,
    pub architecture: String,
    pub repos: Vec<RepoConfig>,
    pub siglevel: Vec<String>,
    pub local_siglevel: Vec<String>,
    pub gpgdir: PathBuf,
    pub cachedirs: Vec<PathBuf>,
    pub hookdirs: Vec<PathBuf>,
    pub ignorepkgs: Vec<String>,
    pub ignoregroups: Vec<String>,
    pub noextract: Vec<String>,
    pub noupgrade: Vec<String>,
    pub download_user: Option<String>,
    pub xfer_command: Option<String>,
}
impl AlpmConfig {
    pub fn load(path: &Path, io: &CommandRunner) -> AppResult<Self> {
        let spec = CommandSpec::new("/usr/bin/pacman-conf").args([
            "--config".into(),
            path.to_string_lossy().into_owned(),
            "--verbose".into(),
        ]);
        let result = io.run(&spec)?;
        if result.code != 0 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "No se pudo leer la configuración de pacman",
            ));
        }
        let mut cfg = Self {
            root: "/".into(),
            dbpath: "/var/lib/pacman".into(),
            architecture: "x86_64".into(),
            repos: vec![],
            siglevel: vec![],
            local_siglevel: vec![],
            gpgdir: "/etc/pacman.d/gnupg".into(),
            cachedirs: vec![],
            hookdirs: vec![],
            ignorepkgs: vec![],
            ignoregroups: vec![],
            noextract: vec![],
            noupgrade: vec![],
            download_user: None,
            xfer_command: None,
        };
        let mut repo = None;
        for line in result.stdout.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') {
                let name = &line[1..line.len() - 1];
                if name == "options" {
                    repo = None;
                } else {
                    cfg.repos.push(RepoConfig {
                        name: name.into(),
                        servers: vec![],
                        siglevel: vec![],
                        usage: vec![],
                    });
                    repo = Some(cfg.repos.len() - 1);
                }
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let val = val.trim();
            let words = || val.split_whitespace().map(String::from).collect::<Vec<_>>();
            if let Some(i) = repo {
                let r = &mut cfg.repos[i];
                match key {
                    "Server" => r.servers.push(val.into()),
                    "SigLevel" => r.siglevel.extend(words()),
                    "Usage" => r.usage.extend(words()),
                    _ => {}
                }
            } else {
                match key {
                    "RootDir" => cfg.root = val.into(),
                    "DBPath" => cfg.dbpath = val.into(),
                    "Architecture" => cfg.architecture = val.into(),
                    "SigLevel" => cfg.siglevel.extend(words()),
                    "LocalFileSigLevel" => cfg.local_siglevel.extend(words()),
                    "GPGDir" => cfg.gpgdir = val.into(),
                    "CacheDir" => cfg.cachedirs.push(val.into()),
                    "HookDir" => cfg.hookdirs.push(val.into()),
                    "IgnorePkg" => cfg.ignorepkgs.extend(words()),
                    "IgnoreGroup" => cfg.ignoregroups.extend(words()),
                    "NoExtract" => cfg.noextract.extend(words()),
                    "NoUpgrade" => cfg.noupgrade.extend(words()),
                    "DownloadUser" => cfg.download_user = Some(val.into()),
                    "XferCommand" => cfg.xfer_command = Some(val.into()),
                    _ => {}
                }
            }
        }
        Ok(cfg)
    }
}
