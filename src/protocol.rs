use crate::{domain::*, paths::Paths, store::Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const MAX_REQUEST: usize = 1_048_576;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Inventory,
    Status,
    EngineStatus,
    Check,
    Plan,
    Apply,
    Selection,
    History,
    Reopen,
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
pub struct ReopenPayload {
    pub operation_id: OperationId,
    pub app_id: AppId,
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
        Action::Reopen => {
            let _: ReopenPayload = serde_json::from_value(req.payload.clone())?;
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
    let result = execute(&req, paths);
    Ok(Response {
        protocol_version: 1,
        request_id: req.request_id,
        result: result.as_ref().ok().cloned(),
        error: result.err(),
    })
}
fn execute(req: &Request, paths: &Paths) -> AppResult<Value> {
    let store = Store::new(paths.clone());
    match &req.action {
        Action::EngineStatus => {
            crate::engine::verify_helper(paths)?;
            Ok(serde_json::json!({"ready":true,"metadata":crate::engine::metadata()}))
        }
        Action::PrepareEngine => Ok(serde_json::to_value(crate::install::prepare_engine(
            &paths.plugin,
            paths,
            &crate::helper::SilentSink,
        )?)?),
        Action::RemovePlugin => Ok(serde_json::to_value(crate::install::remove_plugin(paths)?)?),
        Action::Status => {
            crate::jobs::recovery::recover(paths)?;
            let mut state = store.load()?;
            let policy = crate::policy::Policy::for_paths(paths)?;
            policy.filter_report(&mut state.report, &state.config)?;
            Ok(serde_json::to_value(state.report)?)
        }
        Action::Inventory => {
            let facts = crate::alpm::system_packages()?;
            let apps = crate::inventory::discover(
                paths,
                &facts,
                &crate::io::command::CommandRunner::default(),
            )?;
            let policy = crate::policy::Policy::for_paths(paths)?;
            let report = store.update(|state| {
                crate::jobs::refresh::merge_inventory(&mut state.report, apps);
                policy.filter_report(&mut state.report, &state.config)?;
                if !state.config.initialized {
                    state.config.selected = state.report.selected.clone();
                    state.config.initialized = true;
                }
                Ok(state.report.clone())
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Action::Selection => {
            let policy = crate::policy::Policy::for_paths(paths)?;
            let data: SelectionPayload = serde_json::from_value(req.payload.clone())?;
            let report = store.update(|state| {
                let visible = policy.visible(&state.report.apps);
                if data
                    .selected
                    .iter()
                    .any(|id| !visible.iter().any(|a| &a.id == id))
                {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "La selección contiene aplicaciones excluidas o ausentes",
                    ));
                }
                state.config.selected = data.selected;
                state.config.initialized = true;
                policy.filter_report(&mut state.report, &state.config)?;
                Ok(state.report.clone())
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Action::Check => {
            let data: CheckPayload = serde_json::from_value(req.payload.clone())?;
            let state = store.load()?;
            if !crate::should_refresh(state.report.checked_at, crate::now(), data.force)
                && !state.report.checks.iter().any(|c| c.stale)
            {
                return Ok(serde_json::to_value(state.report)?);
            }
            let policy = crate::policy::Policy::for_paths(paths)?;
            let checked = crate::jobs::refresh::refresh(
                &state.report,
                &policy,
                &crate::providers::ProviderContext::new(paths.clone()),
            )?;
            let report = store.update(|state| {
                let operation = state.report.operation.clone();
                state.report = checked;
                state.report.operation = operation;
                policy.filter_report(&mut state.report, &state.config)?;
                Ok(state.report.clone())
            })?;
            Ok(serde_json::to_value(report)?)
        }
        Action::Plan => {
            let data: SelectionPayload = serde_json::from_value(req.payload.clone())?;
            let (plan, recipes) = crate::jobs::refresh::prepare_plan(paths, &data.selected)?;
            let recipes=recipes.iter().map(|r|serde_json::json!({"package_base":r.package_base,"commit":r.commit,"digest":r.digest,"diff":r.diff,"dependencies":r.dependencies,"files":r.files.iter().map(|f|serde_json::json!({"path":f.path,"sha256":f.sha256,"content":String::from_utf8_lossy(&f.bytes)})).collect::<Vec<_>>()})).collect::<Vec<_>>();
            Ok(serde_json::json!({"plan":plan,"recipes":recipes}))
        }
        Action::Apply => {
            let data: ApplyPayload = serde_json::from_value(req.payload.clone())?;
            let state = store.load()?;
            let plan = state
                .plans
                .get(&data.plan_id)
                .ok_or_else(|| AppError::new(ErrorCode::PlanChanged, "Plan desconocido"))?;
            if data.digest != plan.digest {
                return Err(AppError::new(
                    ErrorCode::PlanChanged,
                    "El alcance aprobado cambió",
                ));
            }
            let id = crate::jobs::launch::submit(plan, paths)?;
            Ok(serde_json::json!({"operation_id":id,"accepted":true}))
        }
        Action::Operation => {
            let data: OperationPayload = serde_json::from_value(req.payload.clone())?;
            if let Some(value) = data.decision {
                let decision: crate::jobs::control::UserDecision = serde_json::from_value(value)?;
                crate::jobs::control::respond(&data.operation_id, &decision, paths)?;
            }
            let job = crate::jobs::load_job(&data.operation_id, paths)?;
            Ok(
                serde_json::json!({"operation":job.operation,"plan":job.plan,"requested_plan":job.requested_plan,"pending":job.pending,"report":job.report,"created_at":job.created_at}),
            )
        }
        Action::Reopen => {
            let data: ReopenPayload = serde_json::from_value(req.payload.clone())?;
            crate::jobs::worker::reopen(&data.operation_id, &data.app_id, paths)?;
            Ok(serde_json::json!({"reopened":true}))
        }
        Action::History => Ok(serde_json::to_value(store.load()?.operations)?),
        _ => Err(AppError::new(
            ErrorCode::Unsupported,
            "Acción todavía no disponible",
        )),
    }
}
