use super::{bindings as ffi, config::AlpmConfig, cstring, ctext, list_ptrs, verify_abi};
use crate::domain::*;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    ptr::NonNull,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFacts {
    pub name: String,
    pub version: String,
    pub explicit: bool,
    pub files: Vec<String>,
    pub url: String,
    pub depends: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    pub groups: Vec<String>,
    pub ignored: bool,
}
pub struct AlpmReader {
    pub(crate) handle: NonNull<ffi::alpm_handle_t>,
    pub config: AlpmConfig,
}
impl Drop for AlpmReader {
    fn drop(&mut self) {
        // The handle uniquely owns all database objects and is no longer used.
        unsafe {
            ffi::alpm_release(self.handle.as_ptr());
        }
    }
}
impl AlpmReader {
    pub fn open(config: &AlpmConfig, dbpath: &Path) -> AppResult<Self> {
        verify_abi()?;
        let root = cstring(&config.root.to_string_lossy())?;
        let db = cstring(&dbpath.to_string_lossy())?;
        let mut err = 0;
        let handle =
            NonNull::new(unsafe { ffi::alpm_initialize(root.as_ptr(), db.as_ptr(), &mut err) })
                .ok_or_else(|| {
                    AppError::new(
                        ErrorCode::InvalidInput,
                        format!("No se pudo abrir la base de paquetes ({err})"),
                    )
                })?;
        Ok(Self {
            handle,
            config: config.clone(),
        })
    }
    pub fn installed(&self) -> AppResult<Vec<PackageFacts>> {
        // Database/package pointers are borrowed from this live handle; copied into owned DTOs.
        unsafe {
            let db = ffi::alpm_get_localdb(self.handle.as_ptr());
            if db.is_null() {
                return Err(AppError::new(ErrorCode::InvalidInput, "Base local ausente"));
            }
            let mut result = Vec::new();
            for p in list_ptrs(ffi::alpm_db_get_pkgcache(db)) {
                let pkg = p.cast::<ffi::alpm_pkg_t>();
                let name = ctext(ffi::alpm_pkg_get_name(pkg));
                let group_ptrs = list_ptrs(ffi::alpm_pkg_get_groups(pkg));
                let groups = group_ptrs
                    .into_iter()
                    .map(|x| ctext(x.cast()))
                    .collect::<Vec<_>>();
                let filelist = ffi::alpm_pkg_get_files(pkg);
                let mut files = Vec::new();
                if !filelist.is_null() && (*filelist).count > 0 {
                    for f in std::slice::from_raw_parts((*filelist).files, (*filelist).count) {
                        files.push(ctext(f.name));
                    }
                }
                let depends = dependency_strings(ffi::alpm_pkg_get_depends(pkg));
                result.push(PackageFacts {
                    name: name.clone(),
                    version: ctext(ffi::alpm_pkg_get_version(pkg)),
                    explicit: ffi::alpm_pkg_get_reason(pkg) == ffi::ALPM_PKG_REASON_EXPLICIT,
                    files,
                    url: ctext(ffi::alpm_pkg_get_url(pkg)),
                    depends,
                    provides: dependency_strings(ffi::alpm_pkg_get_provides(pkg)),
                    ignored: self
                        .config
                        .ignorepkgs
                        .iter()
                        .any(|p| crate::providers::arch::glob_match(p, &name))
                        || groups.iter().any(|g| {
                            self.config
                                .ignoregroups
                                .iter()
                                .any(|p| crate::providers::arch::glob_match(p, g))
                        }),
                    groups,
                });
            }
            Ok(result)
        }
    }
    pub fn owner(&self, path: &Path) -> AppResult<Option<String>> {
        let relative = path
            .strip_prefix(&self.config.root)
            .unwrap_or(path)
            .to_string_lossy();
        Ok(self
            .installed()?
            .into_iter()
            .find(|p| p.files.iter().any(|f| f == relative.as_ref()))
            .map(|p| p.name))
    }
    pub fn available(&self, sync_path: &Path) -> AppResult<Vec<AvailablePackage>> {
        let mut result = Vec::new();
        // Registration only reads the supplied sync directory; no network/db update is called.
        for repo in &self.config.repos {
            let name = cstring(&repo.name)?;
            let existing = unsafe {
                list_ptrs(ffi::alpm_get_syncdbs(self.handle.as_ptr()))
                    .into_iter()
                    .find(|p| ctext(ffi::alpm_db_get_name((*p).cast())) == repo.name)
            };
            let db = existing.map(|p| p.cast()).unwrap_or_else(|| unsafe {
                ffi::alpm_register_syncdb(self.handle.as_ptr(), name.as_ptr(), 0)
            });
            if db.is_null() {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Repositorio inválido",
                ));
            }
            if !sync_path.join(format!("{}.db", repo.name)).exists() {
                continue;
            }
            unsafe {
                for p in list_ptrs(ffi::alpm_db_get_pkgcache(db)) {
                    let pkg = p.cast::<ffi::alpm_pkg_t>();
                    result.push(AvailablePackage {
                        name: ctext(ffi::alpm_pkg_get_name(pkg)),
                        version: ctext(ffi::alpm_pkg_get_version(pkg)),
                        repository: repo.name.clone(),
                        download_bytes: ffi::alpm_pkg_get_size(pkg).max(0) as u64,
                        filename: ctext(ffi::alpm_pkg_get_filename(pkg)),
                        sha256: ctext(ffi::alpm_pkg_get_sha256sum(pkg)),
                        depends: dependency_strings(ffi::alpm_pkg_get_depends(pkg)),
                        provides: dependency_strings(ffi::alpm_pkg_get_provides(pkg)),
                        files: vec![],
                    });
                }
            }
        }
        Ok(result)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailablePackage {
    pub name: String,
    pub version: String,
    pub repository: String,
    pub download_bytes: u64,
    pub filename: String,
    pub sha256: String,
    pub depends: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    pub files: Vec<String>,
}
unsafe fn dependency_strings(list: *mut ffi::alpm_list_t) -> Vec<String> {
    unsafe {
        list_ptrs(list)
            .iter()
            .map(|d| {
                // libalpm returns a newly allocated dependency expression; copy then free.
                let raw = ffi::alpm_dep_compute_string((*d).cast());
                let value = ctext(raw);
                ffi::free(raw.cast());
                value
            })
            .collect()
    }
}
pub fn system_packages() -> AppResult<Vec<PackageFacts>> {
    let cfg = AlpmConfig::load(
        Path::new("/etc/pacman.conf"),
        &crate::io::command::CommandRunner::default(),
    )?;
    AlpmReader::open(&cfg, &cfg.dbpath)?.installed()
}
pub fn compiled_db_path() -> PathBuf {
    "/var/lib/pacman".into()
}
