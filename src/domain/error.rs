use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    ProtocolMismatch,
    InvalidInput,
    Unsupported,
    IoError,
    NetworkError,
    Busy,
    CoverageNeedsReview,
    PlanExpired,
    PlanChanged,
    AuthCancelled,
    AuthUnavailable,
    AbiIncompatible,
    NeedsEnginePreparation,
    UnsafeArtifact,
    TransactionFailed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}
impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for AppError {}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::IoError, e.to_string())
    }
}
impl From<serde_json::Error> for AppError {
    fn from(_: serde_json::Error) -> Self {
        Self::new(ErrorCode::InvalidInput, "Datos JSON inválidos")
    }
}
pub type AppResult<T> = Result<T, AppError>;
