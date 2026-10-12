use omarchy_app_updates::{domain::*, paths::Paths, protocol::*};
use std::io::Read;
fn main() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/nonexistent".into());
    let mut bytes = Vec::new();
    let result = std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(AppError::from)
        .and_then(|_| decode_request(&bytes))
        .and_then(|r| dispatch(r, &Paths::for_home(home)));
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
