mod app;
mod check;
mod error;
mod operation;
mod plan;
pub use app::*;
pub use check::*;
pub use error::*;
pub use operation::*;
pub use plan::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! identity {
    ($($name:ident),+) => {$ (
        #[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl From<&str> for $name { fn from(s:&str)->Self { Self(s.to_owned()) } }
        impl From<String> for $name { fn from(s:String)->Self { Self(s) } }
        impl std::fmt::Display for $name {
            fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { self.0.fmt(f) }
        }
    )+};
}
identity!(AppId, TargetId, PlanId, OperationId);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Coverage {
    Verified,
    NeedsReview {
        reason: String,
        unknown: Vec<String>,
    },
}
impl Default for Coverage {
    fn default() -> Self {
        Self::NeedsReview {
            reason: "Inventario pendiente".into(),
            unknown: vec![],
        }
    }
}
impl Coverage {
    pub fn verified(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    pub schema_version: u32,
    pub selected: Vec<AppId>,
    #[serde(default)]
    pub initialized: bool,
    #[serde(default)]
    pub presentation: BTreeMap<String, serde_json::Value>,
}
impl Default for UserConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            selected: vec![],
            initialized: false,
            presentation: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub apps: Vec<App>,
    pub checks: Vec<Check>,
    pub selected: Vec<AppId>,
    pub coverage: Coverage,
    pub operation: Option<Operation>,
    pub checked_at: Option<u64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub config: UserConfig,
    pub report: Report,
    pub plans: BTreeMap<PlanId, UpdatePlan>,
    pub operations: BTreeMap<OperationId, Operation>,
}
