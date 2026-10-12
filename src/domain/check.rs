use super::{TargetId, Version};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Current,
    UpdateAvailable,
    PublisherAhead,
    InstalledAhead,
    Unknown,
    Unsupported,
    WebAutomatic,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub target_id: TargetId,
    pub checked_at: u64,
    pub installed: Option<Version>,
    pub available: Option<Version>,
    pub published: Option<Version>,
    pub state: CheckState,
    pub stale: bool,
    pub source: Option<String>,
    pub details: Vec<String>,
}
