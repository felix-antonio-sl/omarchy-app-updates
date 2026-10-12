use std::path::PathBuf;
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub plugin: PathBuf,
    pub helper: PathBuf,
    pub pacman_conf: PathBuf,
    pub system_db: PathBuf,
    pub runtime_dir: PathBuf,
}
impl Paths {
    pub fn for_home(home: PathBuf) -> Self {
        Self {
            config: home.join(".config/omarchy-app-updates"),
            state: home.join(".local/state/omarchy-app-updates"),
            plugin: home.join(".config/omarchy/plugins/korvo.app-updates"),
            helper: "/usr/lib/omarchy-app-updates/app-updates-helper".into(),
            pacman_conf: "/etc/pacman.conf".into(),
            system_db: "/var/lib/pacman".into(),
            runtime_dir: std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/state/omarchy-app-updates/runtime")),
            home,
        }
    }
}
