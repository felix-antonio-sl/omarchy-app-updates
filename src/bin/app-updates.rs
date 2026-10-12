use omarchy_app_updates::{domain::*, paths::Paths, protocol::*};
use std::io::Read;
fn main() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/nonexistent".into());
    let paths = Paths::for_home(home);
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args == ["--version"] {
        println!(
            "{}",
            serde_json::to_string(&omarchy_app_updates::engine::metadata()).unwrap()
        );
        return;
    }
    if !args.is_empty() {
        let result: AppResult<serde_json::Value> = match args.as_slice() {
            [flag, id] if flag == "--worker" => {
                omarchy_app_updates::jobs::worker::run_worker(&OperationId(id.clone()), &paths)
                    .map(|_| serde_json::json!({"worker_finished":true}))
            }
            [flag, source] if flag == "--finish-engine" => {
                omarchy_app_updates::install::finish_engine(std::path::Path::new(source), &paths)
                    .and_then(|r| Ok(serde_json::to_value(r)?))
            }
            [flag, source] if flag == "--install-source" => {
                omarchy_app_updates::install::install_plugin(std::path::Path::new(source), &paths)
                    .and_then(|r| Ok(serde_json::to_value(r)?))
            }
            _ => Err(AppError::new(
                ErrorCode::InvalidInput,
                "Argumentos no admitidos",
            )),
        };
        let response = Response {
            protocol_version: 1,
            request_id: "direct".into(),
            result: result.as_ref().ok().cloned(),
            error: result.err(),
        };
        println!("{}", serde_json::to_string(&response).unwrap());
        if response.error.is_some() {
            std::process::exit(1);
        }
        return;
    }
    let mut bytes = Vec::new();
    let result = std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(AppError::from)
        .and_then(|_| decode_request(&bytes))
        .and_then(|r| dispatch(r, &paths));
    match result {
        Ok(r) => {
            println!("{}", serde_json::to_string(&r).unwrap());
            if r.error.is_some() {
                std::process::exit(1);
            }
        }
        Err(e) => {
            println!(
                "{}",
                serde_json::to_string(&Response {
                    protocol_version: 1,
                    request_id: "invalid".into(),
                    result: None,
                    error: Some(e)
                })
                .unwrap()
            );
            std::process::exit(1);
        }
    }
}
