pub mod arch;
pub mod aur;
use crate::{
    domain::*,
    io::{command::CommandRunner, http::HttpClient},
    paths::Paths,
};
#[derive(Clone)]
pub struct ProviderContext {
    pub paths: Paths,
    pub commands: CommandRunner,
    pub http: HttpClient,
    pub now: u64,
}
impl ProviderContext {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            commands: CommandRunner::default(),
            http: HttpClient::default(),
            now: crate::now(),
        }
    }
}
pub trait Provider: Send + Sync {
    fn check(&self, target: &Target, ctx: &ProviderContext) -> AppResult<Check>;
}
pub fn check_base(target: &Target, ctx: &ProviderContext, state: CheckState) -> Check {
    Check {
        target_id: target.id.clone(),
        checked_at: ctx.now,
        installed: target.installed.clone(),
        available: None,
        published: None,
        state,
        stale: false,
        source: None,
        details: vec![],
    }
}
pub fn version_state(installed: Option<&Version>, available: &Version) -> AppResult<CheckState> {
    let Some(installed) = installed else {
        return Ok(CheckState::Unknown);
    };
    let compare = crate::alpm::arch_compare(&installed.raw, &available.raw)?;
    Ok(match compare {
        std::cmp::Ordering::Less => CheckState::UpdateAvailable,
        std::cmp::Ordering::Equal => CheckState::Current,
        std::cmp::Ordering::Greater => CheckState::InstalledAhead,
    })
}
