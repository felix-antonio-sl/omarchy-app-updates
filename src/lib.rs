pub mod alpm;
pub mod domain;
pub mod engine;
pub mod execution;
mod flatpak_native;
pub mod helper;
pub mod install;
pub mod inventory;
pub mod io;
pub mod jobs;
pub mod paths;
pub mod planner;
pub mod policy;
pub mod protocol;
pub mod providers;
pub mod store;

pub fn should_refresh(checked_at: Option<u64>, now: u64, force: bool) -> bool {
    force || checked_at.is_none_or(|t| now < t || now - t > 21_600)
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
