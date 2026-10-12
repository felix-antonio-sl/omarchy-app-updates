use super::{AppError, AppId, OperationId, TargetId, UpdatePlan, Version};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    Queued,
    Downloading,
    Building,
    AwaitingAuth,
    AwaitingDecision,
    Installing,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Updated,
    Current,
    NeedsReopen,
    Failed,
    Skipped,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetResult {
    pub target_id: TargetId,
    pub outcome: Outcome,
    pub old_version: Option<Version>,
    pub new_version: Option<Version>,
    pub error: Option<AppError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub id: OperationId,
    pub plan_digest: String,
    pub phase: Phase,
    pub unit: Option<String>,
    pub events: Vec<OperationEvent>,
    pub results: Vec<TargetResult>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEvent {
    pub operation_id: OperationId,
    pub seq: u64,
    pub phase: Phase,
    pub message: String,
    pub progress: Option<f64>,
    pub detail: Option<EventDetail>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EventDetail {
    PlanRevised(UpdatePlan),
    Question {
        id: String,
        kind: String,
        choices: Vec<String>,
    },
    NeedsReopen(AppId),
}
