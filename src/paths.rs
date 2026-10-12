use std::path::PathBuf;
#[derive(Debug, Clone)]
pub struct Paths {
    pub application_dirs: Vec<PathBuf>,
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
        let mut application_dirs = vec![
            home.join(".local/share/applications"),
            home.join(".local/share/flatpak/exports/share/applications"),
        ];
        if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
            application_dirs.insert(0, PathBuf::from(d).join("applications"));
        }
        let system =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        application_dirs.extend(
            system
                .split(':')
                .filter(|s| !s.is_empty())
                .map(|s| PathBuf::from(s).join("applications")),
        );
        application_dirs.push("/var/lib/flatpak/exports/share/applications".into());
        Self {
            application_dirs,
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
