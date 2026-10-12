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

#[test]
fn administrative_policy_reads_only_bounded_caller_owned_regular_files() {
    use omarchy_app_updates::helper::administration::{policy_for_caller, read_user_data};
    use omarchy_app_updates::helper::validate::Caller;
    let d = tempfile::tempdir().unwrap();
    let uid = fs::metadata(d.path()).unwrap().uid();
    let caller = Caller {
        uid,
        gid: uid,
        home: d.path().into(),
    };
    let (_, _, _, paths) = common::fixture(d.path());
    assert!(
        policy_for_caller(&caller, &paths)
            .unwrap()
            .coverage
            .verified()
    );
    let source = d.path().join("source");
    fs::write(&source, b"public source").unwrap();
    assert_eq!(
        read_user_data(&source, uid, 100).unwrap().unwrap(),
        b"public source"
    );
    assert!(read_user_data(&source, uid + 1, 100).is_err());
    assert!(read_user_data(&source, uid, 5).is_err());
    let link = d.path().join("private-link");
    symlink("/etc/shadow", &link).unwrap();
    assert_eq!(
        read_user_data(&link, uid, 100).unwrap_err().code,
        ErrorCode::UnsafeArtifact
    );
}
#[test]
fn sync_promotion_is_reversible_and_never_follows_database_links() {
    use omarchy_app_updates::helper::administration::SyncPromotion;
    let d = tempfile::tempdir().unwrap();
    let uid = fs::metadata(d.path()).unwrap().uid();
    let source = d.path().join("new");
    let dest = d.path().join("system");
    let backup = d.path().join("backup");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("extra.db"), b"old database").unwrap();
    fs::write(source.join("extra.db"), b"new database").unwrap();
    fs::write(source.join("extra.db.sig"), b"new signature").unwrap();
    let mut promotion = SyncPromotion::new(source.clone(), dest.clone(), backup, uid).unwrap();
    promotion.promote().unwrap();
    assert_eq!(fs::read(dest.join("extra.db")).unwrap(), b"new database");
    promotion.restore().unwrap();
    assert_eq!(fs::read(dest.join("extra.db")).unwrap(), b"old database");
    assert!(!dest.join("extra.db.sig").exists());
    fs::remove_file(source.join("extra.db")).unwrap();
    symlink(dest.join("extra.db"), source.join("extra.db")).unwrap();
    let mut p = SyncPromotion::new(source, dest.clone(), d.path().join("backup2"), uid).unwrap();
    assert_eq!(p.promote().unwrap_err().code, ErrorCode::UnsafeArtifact);
    assert_eq!(fs::read(dest.join("extra.db")).unwrap(), b"old database");
}
#[test]
fn user_helper_override_is_rejected_before_graphical_authorization() {
    use omarchy_app_updates::helper::HelperClient;
    let d = tempfile::tempdir().unwrap();
    let (_, _, _, mut paths) = common::fixture(d.path());
    paths.helper = d.path().join("helper");
    fs::write(&paths.helper, b"user code").unwrap();
    fs::set_permissions(&paths.helper, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(HelperClient::start(&"op".into(),&paths),Err(e)if e.code==ErrorCode::UnsafeArtifact)
    );
}
#[test]
fn administrative_factory_cannot_acquire_host_locks_as_an_unprivileged_user() {
    use omarchy_app_updates::{
        alpm::transaction::AlpmTransaction,
        helper::{SilentSink, validate::Caller},
    };
    let d = tempfile::tempdir().unwrap();
    let uid = fs::metadata(d.path()).unwrap().uid();
    if uid == 0 {
        return;
    }
    assert!(
        matches!(AlpmTransaction::system(Caller{uid,gid:uid,home:d.path().into()},"op".into(),std::sync::Arc::new(SilentSink)),Err(e)if e.code==ErrorCode::AuthUnavailable)
    );
}

#[test]
fn replacing_database_without_signature_removes_old_signature_and_restores_both() {
    use omarchy_app_updates::helper::administration::SyncPromotion;
    let d = tempfile::tempdir().unwrap();
    let uid = fs::metadata(d.path()).unwrap().uid();
    let source = d.path().join("new");
    let dest = d.path().join("system");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("extra.db"), b"old").unwrap();
    fs::write(dest.join("extra.db.sig"), b"old signature").unwrap();
    fs::write(source.join("extra.db"), b"new").unwrap();
    let mut p = SyncPromotion::new(source, dest.clone(), d.path().join("backup"), uid).unwrap();
    p.promote().unwrap();
    assert!(!dest.join("extra.db.sig").exists());
    p.restore().unwrap();
    assert_eq!(fs::read(dest.join("extra.db")).unwrap(), b"old");
    assert_eq!(
        fs::read(dest.join("extra.db.sig")).unwrap(),
        b"old signature"
    );
}

#[test]
fn archive_payload_paths_and_unreviewed_migrations_are_rejected() {
    use omarchy_app_updates::helper::administration::{
        validate_migration_payload, validate_payload_paths,
    };
    let d = tempfile::tempdir().unwrap();
    let (_, policy, _) = common::plan(d.path());
    assert!(validate_payload_paths(&["usr/bin/ghostty".into()], &policy).is_ok());
    for bad in [
        "/etc/shadow",
        "usr/../root/.ssh/id_rsa",
        "usr/bin/codex",
        "usr/bin/claude",
    ] {
        assert!(
            validate_payload_paths(&[bad.into()], &policy).is_err(),
            "{bad}"
        );
    }
    assert!(validate_migration_payload(&["usr/bin/ghostty".into()]).is_ok());
    assert_eq!(
        validate_migration_payload(&["usr/share/omarchy/migrations/9999999999.sh".into()])
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
}
#[test]
fn helper_metadata_is_available_without_administrative_authentication() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_app-updates-helper"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(metadata["protocol"], 1);
    assert_eq!(
        metadata["alpm_runtime"],
        omarchy_app_updates::alpm::runtime_version()
    );
    assert!(metadata["source_digest"].as_str().unwrap().len() == 64);
}
