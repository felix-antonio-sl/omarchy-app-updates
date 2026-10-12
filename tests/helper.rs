mod common;
use omarchy_app_updates::{
    domain::*,
    helper::{
        HelperRequest,
        staging::StagingArea,
        validate::{
            validate_admin_config, validate_caller, validate_helper_path,
            validate_resolved_packages,
        },
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};
#[test]
fn unknown_requests_and_user_administrative_configuration_are_rejected() {
    assert!(
        serde_json::from_str::<HelperRequest>(r#"{"action":"execute","command":"echo unsafe"}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<HelperRequest>(r#"{"action":"confirm","operation_id":"op","digest":"abc","pacman_conf":"/home/user/pacman.conf"}"#).is_err());
    assert!(validate_admin_config(std::path::Path::new("/home/user/pacman.conf")).is_err());
    assert!(validate_helper_path(std::path::Path::new("/home/user/helper")).is_err());
}
#[test]
fn root_requires_pkexec_caller_and_uses_passwd_home() {
    let passwd = "korvo:x:1000:1000::/home/korvo:/bin/bash\nroot:x:0:0::/root:/bin/bash\n";
    for uid in [None, Some("0"), Some("-1"), Some("bad"), Some("1001")] {
        assert!(validate_caller(0, uid, passwd).is_err());
    }
    assert!(validate_caller(1000, Some("1000"), passwd).is_err());
    let caller = validate_caller(0, Some("1000"), passwd).unwrap();
    assert_eq!(caller.uid, 1000);
    assert_eq!(caller.home, std::path::Path::new("/home/korvo"));
}
#[test]
fn staging_copies_exact_approved_bytes_and_rejects_links_or_replaced_checksum() {
    let d = tempfile::tempdir().unwrap();
    let uid = fs::metadata(d.path()).unwrap().uid();
    let source = d.path().join("source.pkg.tar.zst");
    let bytes = b"reviewed package";
    fs::write(&source, bytes).unwrap();
    let staging = StagingArea::new(d.path().join("staging"), uid).unwrap();
    let digest = format!("{:x}", Sha256::digest(bytes));
    let staged = staging.copy(&source, &digest, uid).unwrap();
    assert_eq!(fs::read(&staged).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&staged).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let link = d.path().join("link.pkg.tar.zst");
    symlink(&source, &link).unwrap();
    assert_eq!(
        staging.copy(&link, &digest, uid).unwrap_err().code,
        ErrorCode::UnsafeArtifact
    );
    fs::write(&source, b"substituted").unwrap();
    assert_eq!(
        staging.copy(&source, &digest, uid).unwrap_err().code,
        ErrorCode::UnsafeArtifact
    );
    let evil = d.path().join("evil");
    symlink(d.path(), &evil).unwrap();
    assert!(StagingArea::new(evil, uid).is_err());
}
#[test]
fn resolved_packages_are_checked_independently_of_client_selection() {
    let d = tempfile::tempdir().unwrap();
    let (_, mut snapshot, policy, _) = common::fixture(d.path());

    snapshot.packages[0].files = vec!["usr/bin/opencode".into()];
    assert_eq!(
        validate_resolved_packages(&snapshot.packages, &policy)
            .unwrap_err()
            .code,
        ErrorCode::CoverageNeedsReview
    );
    snapshot.packages[0].files.clear();
    snapshot.packages[0].name = "opencode".into();
    assert!(validate_resolved_packages(&snapshot.packages, &policy).is_err());
}

#[test]
fn graphical_authentication_cancellation_is_a_cancelled_error() {
    assert_eq!(
        omarchy_app_updates::helper::client::authorization_error(126).code,
        ErrorCode::AuthCancelled
    );
    assert_eq!(
        omarchy_app_updates::helper::client::authorization_error(127).code,
        ErrorCode::AuthUnavailable
    );
}
#[test]
fn pacman_signature_policy_preserves_scopes_and_rejects_unknown_modes() {
    use omarchy_app_updates::alpm::transaction::signature_level;
    let words = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    let base = signature_level(
        &words("PackageRequired PackageTrustedOnly DatabaseOptional DatabaseTrustedOnly"),
        0,
    )
    .unwrap();
    assert_eq!(base, 1 | 1024 | 2048);
    assert_eq!(
        signature_level(&words("PackageNever DatabaseNever"), base).unwrap(),
        0
    );
    assert_eq!(
        signature_level(&words("PackageOptional PackageTrustedOnly"), 0).unwrap(),
        3
    );
    assert!(signature_level(&words("PackageSomethingNew"), base).is_err());
}
