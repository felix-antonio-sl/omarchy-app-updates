pub mod domain;
pub mod paths;
pub mod protocol;
pub mod store;

pub fn should_refresh(checked_at: Option<u64>, now: u64, force: bool) -> bool {
    force || checked_at.is_none_or(|t| now < t || now - t > 21_600)
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
