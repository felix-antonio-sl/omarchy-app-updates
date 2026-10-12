pub mod client;
pub mod control;
pub mod session;
pub use client::HelperClient;
pub mod administration;
pub mod staging;
pub(crate) mod system;
pub mod validate;
use crate::{domain::*, execution::EventSink};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct QuestionId(pub String);
impl From<&str> for QuestionId {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}
impl From<String> for QuestionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionValue {
    Approve,
    Reject,
    SelectProvider(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperQuestion {
    pub id: QuestionId,
    pub kind: String,
    pub message: String,
    pub choices: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperRequest {
    Prepare {
        operation_id: OperationId,
        plan: UpdatePlan,
    },
    Confirm {
        operation_id: OperationId,
        digest: String,
    },
    Answer {
        operation_id: OperationId,
        question_id: QuestionId,
        answer: DecisionValue,
    },
    Abort {
        operation_id: OperationId,
    },
}
impl HelperRequest {
    pub fn operation_id(&self) -> &OperationId {
        match self {
            Self::Prepare { operation_id, .. }
            | Self::Confirm { operation_id, .. }
            | Self::Answer { operation_id, .. }
            | Self::Abort { operation_id } => operation_id,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HelperEvent {
    Prepared {
        operation_id: OperationId,
        plan: UpdatePlan,
    },
    RevisedPlan {
        operation_id: OperationId,
        plan: UpdatePlan,
    },
    Question {
        operation_id: OperationId,
        question: HelperQuestion,
        digest: String,
    },
    Progress {
        data: OperationEvent,
    },
    Result {
        operation_id: OperationId,
        results: Vec<TargetResult>,
    },
    Error {
        operation_id: Option<OperationId>,
        error: AppError,
    },
}
pub fn write_event(event: &HelperEvent, output: &mut dyn std::io::Write) -> AppResult<()> {
    let mut line = serde_json::to_vec(event)?;
    line.push(b'\n');
    output.write_all(&line)?;
    output.flush()?;
    Ok(())
}
pub struct SilentSink;
impl EventSink for SilentSink {
    fn send(&self, _: OperationEvent) -> AppResult<()> {
        Ok(())
    }
}
