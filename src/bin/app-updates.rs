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
        let result = if args.len() == 2 && args[0] == "--worker" {
            omarchy_app_updates::jobs::worker::run_worker(&OperationId(args[1].clone()), &paths)
        } else {
            Err(AppError::new(
                ErrorCode::InvalidInput,
                "Argumentos no admitidos",
            ))
        };
        if let Err(error) = result {
            println!(
                "{}",
                serde_json::to_string(&Response {
                    protocol_version: 1,
                    request_id: "worker".into(),
                    result: None,
                    error: Some(error)
                })
                .unwrap()
            );
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
