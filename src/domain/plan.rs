use super::{AppId, FlatpakInstall, PlanId, TargetId};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePlan {
    #[serde(default)]
    pub decisions: Vec<PlanDecision>,
    #[serde(default)]
    pub selected: Vec<AppId>,
    #[serde(default)]
    pub repository_digest: String,
    #[serde(default)]
    pub notices: Vec<String>,
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
    #[serde(default)]
    pub flatpak_changes: Vec<FlatpakChange>,
    pub kind: StepKind,
    pub targets: Vec<TargetId>,
    pub package_changes: Vec<PackageChange>,
    pub downloads: Vec<Download>,
    pub requires_auth: bool,
    pub reopen: Vec<AppId>,
    pub aur_recipe: Option<AurRecipeRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlatpakChange {
    pub installation: FlatpakInstall,
    pub reference: String,
    pub remote: String,
    pub commit: String,
    pub previous_commit: Option<String>,
    pub operation_type: u32,
    pub download_bytes: u64,
    pub metadata: String,
    pub old_metadata: String,
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
    #[serde(default)]
    pub action: PackageAction,
    pub name: String,
    pub old_version: Option<String>,
    pub new_version: String,
    pub repository: Option<String>,
    pub reason: String,
    pub download_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PackageAction {
    #[default]
    Upgrade,
    Install,
    Remove,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDecision {
    pub question_id: String,
    pub kind: String,
    pub value: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub url: String,
    pub expected_sha256: Option<String>,
    pub bytes: Option<u64>,
}
