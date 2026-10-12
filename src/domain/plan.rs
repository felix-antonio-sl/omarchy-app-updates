use super::{AppId, PlanId, TargetId};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlan {
    pub id: PlanId,
    pub created_at: u64,
    pub expires_at: u64,
    pub inventory_digest: String,
    pub policy_digest: String,
    pub steps: Vec<PlanStep>,
    pub digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub kind: StepKind,
    pub targets: Vec<TargetId>,
    pub package_changes: Vec<PackageChange>,
    pub downloads: Vec<Download>,
    pub requires_auth: bool,
    pub reopen: Vec<AppId>,
    pub aur_recipe: Option<AurRecipeRef>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    ArchFull,
    AurBuild,
    FlatpakUpdate,
    TelegramReplace,
    SteamUpdate,
    Reopen,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AurRecipeRef {
    pub package_base: String,
    pub commit: String,
    pub digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageChange {
    pub name: String,
    pub old_version: Option<String>,
    pub new_version: String,
    pub repository: Option<String>,
    pub reason: String,
    pub download_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub url: String,
    pub expected_sha256: Option<String>,
    pub bytes: Option<u64>,
}
