use crate::{domain::*, paths::Paths, store::Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const MAX_REQUEST: usize = 1_048_576;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Inventory,
    Status,
    Check,
    Plan,
    Apply,
    Selection,
    History,
    Operation,
    Worker,
    PrepareEngine,
    RemovePlugin,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol_version: u32,
    pub request_id: String,
    pub action: Action,
    pub payload: Value,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub protocol_version: u32,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionPayload {
    pub selected: Vec<AppId>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckPayload {
    #[serde(default)]
    pub force: bool,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyPayload {
    pub plan_id: PlanId,
    pub digest: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationPayload {
    pub operation_id: OperationId,
    #[serde(default)]
    pub decision: Option<Value>,
}
pub fn decode_request(bytes: &[u8]) -> AppResult<Request> {
    if bytes.len() > MAX_REQUEST {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Solicitud demasiado grande",
        ));
    }
    let req: Request = serde_json::from_slice(bytes)?;
    if req.protocol_version != 1 {
        return Err(AppError::new(
            ErrorCode::ProtocolMismatch,
            "Protocolo incompatible",
        ));
    }
    if req.request_id.is_empty()
        || req.request_id.len() > 80
        || !req
            .request_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Identificador inválido",
        ));
    }
    match req.action {
        Action::Selection | Action::Plan => {
            let _: SelectionPayload = serde_json::from_value(req.payload.clone())?;
        }
        Action::Check => {
            let _: CheckPayload = serde_json::from_value(req.payload.clone())?;
        }
        Action::Apply => {
            let _: ApplyPayload = serde_json::from_value(req.payload.clone())?;
        }
        Action::Operation | Action::Worker => {
            let _: OperationPayload = serde_json::from_value(req.payload.clone())?;
        }
        _ => {
            let _: Empty = serde_json::from_value(req.payload.clone())?;
        }
    }
    Ok(req)
}
pub fn dispatch(req: Request, paths: &Paths) -> AppResult<Response> {
    let result = match req.action {
        Action::Inventory => {
            let facts = crate::alpm::system_packages()?;
            let apps = crate::inventory::discover(
                paths,
                &facts,
                &crate::io::command::CommandRunner::default(),
            )?;
            let store = Store::new(paths.clone());
            let mut state = store.load()?;
            state.report.apps = apps;
            store.save(&state)?;
            Ok(serde_json::to_value(state.report)?)
        }
        Action::Status => Ok(serde_json::to_value(
            Store::new(paths.clone()).load()?.report,
        )?),
        Action::History => Ok(serde_json::to_value(
            Store::new(paths.clone()).load()?.operations,
        )?),
        _ => Err(AppError::new(
            ErrorCode::Unsupported,
            "Acción todavía no disponible",
        )),
    };
    Ok(Response {
        protocol_version: 1,
        request_id: req.request_id,
        result: result.as_ref().ok().cloned(),
        error: result.err(),
    })
}
