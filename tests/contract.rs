use omarchy_app_updates::{
    paths::Paths,
    protocol::{decode_request, dispatch},
    should_refresh,
    store::Store,
};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn rejects_unknown_protocol_and_actions() {
    assert!(
        decode_request(
            br#"{"protocol_version":2,"request_id":"a","action":"status","payload":{}}"#
        )
        .is_err()
    );
    assert!(decode_request(br#"{"protocol_version":1,"request_id":"a","action":"shell","payload":{"command":"touch /tmp/should-not-exist"}}"#).is_err());
    assert!(decode_request(br#"{"protocol_version":1,"request_id":"a","action":"status","payload":{"command":"evil"}}"#).is_err());
}
#[test]
fn response_retains_request_identity() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(dir.path().into());
    let req = decode_request(
        br#"{"protocol_version":1,"request_id":"req-42","action":"status","payload":{}}"#,
    )
    .unwrap();
    let response = dispatch(req, &paths).unwrap();
    assert_eq!(response.request_id, "req-42");
    assert_eq!(response.protocol_version, 1);
}
#[test]
fn state_is_private_atomic_and_recovers_without_destroying_data() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::for_home(dir.path().into());
    let store = Store::new(paths.clone());
    let mut state = store.load().unwrap();
    state.config.selected = vec!["repo:ghostty".into()];
    store.save(&state).unwrap();
    assert_eq!(
        fs::metadata(&paths.state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(paths.state.join("state.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(store.load().unwrap().config.selected[0].0, "repo:ghostty");
    state.config.selected = vec!["repo:audacity".into()];
    store.save(&state).unwrap();
    assert!(paths.state.join("state.json.previous").is_file());
    assert!(!fs::read_dir(&paths.state).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp-")
    }));
    fs::write(paths.state.join("state.json"), b"broken").unwrap();
    assert!(store.load().is_err());
    assert_eq!(fs::read(paths.state.join("state.json")).unwrap(), b"broken");
}
#[test]
fn refresh_clock_edges() {
    assert!(should_refresh(None, 100, false));
    assert!(!should_refresh(Some(0), 21600, false));
    assert!(should_refresh(Some(0), 21601, false));
    assert!(should_refresh(Some(100), 99, false));
    assert!(should_refresh(Some(100), 100, true));
}
