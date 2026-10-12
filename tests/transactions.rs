mod common;
use omarchy_app_updates::{
    alpm::transaction::PackageTransaction,
    domain::*,
    execution::EventSink,
    helper::{
        DecisionValue, HelperEvent, HelperQuestion, HelperRequest, QuestionId,
        session::HelperSession,
    },
    planner::plan_digest,
};
use std::{
    io::Cursor,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
struct Engine {
    commits: Arc<AtomicUsize>,
    releases: Arc<AtomicUsize>,
    revise: bool,
    question: Option<HelperQuestion>,
    answered: bool,
}
impl PackageTransaction for Engine {
    fn prepare(&mut self, approved: &UpdatePlan) -> AppResult<UpdatePlan> {
        if self.question.is_some() && !self.answered {
            return Err(AppError::new(ErrorCode::Busy, "Awaiting decision"));
        }
        let mut p = approved.clone();
        if self.revise {
            p.steps[0].package_changes[0].new_version = "3.0-1".into();
            p.digest = plan_digest(&p);
        }
        Ok(p)
    }
    fn commit(&mut self, _: &str, _: &dyn EventSink) -> AppResult<Vec<TargetResult>> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    }
    fn release(&mut self) -> AppResult<()> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn pending_question(&self) -> Option<HelperQuestion> {
        if self.answered {
            None
        } else {
            self.question.clone()
        }
    }
    fn answer(&mut self, id: &QuestionId, value: &DecisionValue) -> AppResult<()> {
        if self.question.as_ref().is_none_or(|q| q.id != *id) || *value != DecisionValue::Approve {
            return Err(AppError::new(ErrorCode::InvalidInput, "wrong answer"));
        }
        self.answered = true;
        Ok(())
    }
}
fn input(requests: &[HelperRequest]) -> Cursor<Vec<u8>> {
    let mut v = vec![];
    for r in requests {
        v.extend(serde_json::to_vec(r).unwrap());
        v.push(b'\n');
    }
    Cursor::new(v)
}
fn engine(
    revise: bool,
    question: Option<HelperQuestion>,
) -> (Engine, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let c = Arc::new(AtomicUsize::new(0));
    let r = Arc::new(AtomicUsize::new(0));
    (
        Engine {
            commits: c.clone(),
            releases: r.clone(),
            revise,
            question,
            answered: false,
        },
        c,
        r,
    )
}
#[test]
fn prepare_does_not_install_and_eof_releases_transaction() {
    let d = tempfile::tempdir().unwrap();
    let (p, policy, _) = common::plan(d.path());
    let (e, c, r) = engine(false, None);
    let mut session = HelperSession::new(e, policy, 100);
    let mut output = vec![];
    assert!(
        session
            .run(
                &mut input(&[HelperRequest::Prepare {
                    operation_id: "op".into(),
                    plan: p
                }]),
                &mut output
            )
            .is_err()
    );
    assert_eq!(c.load(Ordering::SeqCst), 0);
    assert_eq!(r.load(Ordering::SeqCst), 1);
    let events = String::from_utf8(output).unwrap();
    assert!(events.contains("prepared"));
    assert!(!events.contains("\"result\""));
}
#[test]
fn revised_plan_cannot_confirm_using_previous_digest() {
    let d = tempfile::tempdir().unwrap();
    let (p, policy, _) = common::plan(d.path());
    let (e, c, r) = engine(true, None);
    let mut session = HelperSession::new(e, policy, 100);
    let mut output = vec![];
    let digest = p.digest.clone();
    let err = session
        .run(
            &mut input(&[
                HelperRequest::Prepare {
                    operation_id: "op".into(),
                    plan: p,
                },
                HelperRequest::Confirm {
                    operation_id: "op".into(),
                    digest,
                },
            ]),
            &mut output,
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PlanChanged);
    assert_eq!(c.load(Ordering::SeqCst), 0);
    assert_eq!(r.load(Ordering::SeqCst), 1);
    assert!(String::from_utf8(output).unwrap().contains("revised_plan"));
}
#[test]
fn operation_id_and_question_id_are_both_required() {
    for (other_operation, other_question) in [(true, false), (false, true)] {
        let d = tempfile::tempdir().unwrap();
        let (p, policy, _) = common::plan(d.path());
        let q = HelperQuestion {
            id: "q-conflict".into(),
            kind: "conflict".into(),
            message: "Remove old package?".into(),
            choices: vec!["approve".into(), "reject".into()],
        };
        let (e, c, r) = engine(false, Some(q));
        let mut session = HelperSession::new(e, policy, 100);
        assert!(
            session
                .run(
                    &mut input(&[
                        HelperRequest::Prepare {
                            operation_id: "op".into(),
                            plan: p.clone()
                        },
                        HelperRequest::Answer {
                            operation_id: if other_operation { "other" } else { "op" }.into(),
                            question_id: if other_question {
                                "other"
                            } else {
                                "q-conflict"
                            }
                            .into(),
                            answer: DecisionValue::Approve
                        },
                        HelperRequest::Confirm {
                            operation_id: "op".into(),
                            digest: p.digest
                        }
                    ]),
                    &mut vec![]
                )
                .is_err()
        );
        assert_eq!(c.load(Ordering::SeqCst), 0);
        assert_eq!(r.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn conflicts_replacements_and_keys_never_get_default_approval() {
    for kind in ["conflict", "replace", "import_key"] {
        let d = tempfile::tempdir().unwrap();
        let (p, policy, _) = common::plan(d.path());
        let (e, c, r) = engine(
            false,
            Some(HelperQuestion {
                id: "question".into(),
                kind: kind.into(),
                message: "Review needed".into(),
                choices: vec!["approve".into(), "reject".into()],
            }),
        );
        let mut session = HelperSession::new(e, policy, 100);
        assert!(
            session
                .run(
                    &mut input(&[
                        HelperRequest::Prepare {
                            operation_id: "op".into(),
                            plan: p.clone()
                        },
                        HelperRequest::Confirm {
                            operation_id: "op".into(),
                            digest: p.digest
                        }
                    ]),
                    &mut vec![]
                )
                .is_err()
        );
        assert_eq!(c.load(Ordering::SeqCst), 0);
        assert_eq!(r.load(Ordering::SeqCst), 1);
    }
}
#[test]
fn only_matching_confirm_commits_and_abort_is_not_success() {
    for abort in [true, false] {
        let d = tempfile::tempdir().unwrap();
        let (p, policy, _) = common::plan(d.path());
        let (e, c, r) = engine(false, None);
        let mut session = HelperSession::new(e, policy, 100);
        let second = if abort {
            HelperRequest::Abort {
                operation_id: "op".into(),
            }
        } else {
            HelperRequest::Confirm {
                operation_id: "op".into(),
                digest: p.digest.clone(),
            }
        };
        let mut output = vec![];
        let result = session.run(
            &mut input(&[
                HelperRequest::Prepare {
                    operation_id: "op".into(),
                    plan: p,
                },
                second,
            ]),
            &mut output,
        );
        if abort {
            assert_eq!(result.unwrap_err().code, ErrorCode::AuthCancelled);
        } else {
            result.unwrap();
            let line = String::from_utf8(output)
                .unwrap()
                .lines()
                .last()
                .unwrap()
                .to_owned();
            assert!(matches!(
                serde_json::from_str::<HelperEvent>(&line).unwrap(),
                HelperEvent::Result { .. }
            ));
        }
        assert_eq!(c.load(Ordering::SeqCst), usize::from(!abort));
        assert_eq!(r.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn decision_channel_timeout_releases_without_committing() {
    use omarchy_app_updates::helper::control::{DeadlineRead, DecisionDeadline};
    use std::{io::BufReader, os::unix::net::UnixStream, time::Duration};
    let d = tempfile::tempdir().unwrap();
    let (p, policy, _) = common::plan(d.path());
    let (e, c, r) = engine(false, None);
    let (reader, mut writer) = UnixStream::pair().unwrap();
    use std::io::Write;
    let request = serde_json::to_vec(&HelperRequest::Prepare {
        operation_id: "op".into(),
        plan: p,
    })
    .unwrap();
    writer.write_all(&request).unwrap();
    writer.write_all(b"\n").unwrap();
    let deadline = DecisionDeadline::new(Duration::from_millis(25));
    let mut input = BufReader::new(DeadlineRead::new(reader, deadline.clone()));
    let mut session = HelperSession::new(e, policy, 100).with_deadline(deadline);
    let err = session.run(&mut input, &mut vec![]).unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthCancelled);
    assert_eq!(c.load(Ordering::SeqCst), 0);
    assert_eq!(r.load(Ordering::SeqCst), 1);
}

fn native_fixture(
    home: &std::path::Path,
    virtual_providers: bool,
) -> (
    omarchy_app_updates::alpm::transaction::AlpmTransaction,
    UpdatePlan,
    std::path::PathBuf,
    Vec<u8>,
) {
    use omarchy_app_updates::{
        alpm::{AlpmConfig, transaction::AlpmTransaction},
        io::command::CommandRunner,
    };
    use std::{fs, process::Command};
    let (plan, policy, _) = common::plan(home);
    let root = home.join("root");
    let db = home.join("db");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(db.join("local/ghostty-1.0-1")).unwrap();
    fs::create_dir_all(db.join("sync")).unwrap();
    fs::write(db.join("local/ALPM_DB_VERSION"), "9\n").unwrap();
    let local=b"%NAME%\nghostty\n\n%VERSION%\n1.0-1\n\n%DESC%\nGhostty fixture\n\n%ARCH%\nx86_64\n\n%REASON%\n0\n\n".to_vec();
    fs::write(db.join("local/ghostty-1.0-1/desc"), &local).unwrap();
    fs::write(
        db.join("local/ghostty-1.0-1/files"),
        "%FILES%\nusr/bin/ghostty\n\n",
    )
    .unwrap();
    let repo = home.join("repo");
    let mut names = vec![];
    let variants = if virtual_providers {
        vec![
            ("ghostty", "fixture-virtual>=2", ""),
            ("impl-one", "", "fixture-virtual=2.0"),
            ("impl-two", "", "fixture-virtual=2.0"),
        ]
    } else {
        vec![("ghostty", "fixture-lib>=2", ""), ("fixture-lib", "", "")]
    };
    for (name, dep, provide) in variants {
        let directory = format!("{name}-2.0-1");
        let folder = repo.join(&directory);
        fs::create_dir_all(&folder).unwrap();
        let mut desc = format!(
            "%NAME%\n{name}\n\n%VERSION%\n2.0-1\n\n%DESC%\nNative package fixture\n\n%ARCH%\nx86_64\n\n%FILENAME%\n{name}.pkg.tar.zst\n\n%CSIZE%\n1\n\n%ISIZE%\n1\n\n"
        );
        if !dep.is_empty() {
            desc += &format!("%DEPENDS%\n{dep}\n\n");
        }
        if !provide.is_empty() {
            desc += &format!("%PROVIDES%\n{provide}\n\n");
        }
        fs::write(folder.join("desc"), desc).unwrap();
        names.push(directory);
    }
    assert!(
        Command::new("/usr/bin/tar")
            .arg("-czf")
            .arg(db.join("sync/extra.db"))
            .arg("-C")
            .arg(&repo)
            .args(names)
            .status()
            .unwrap()
            .success()
    );
    let config = home.join("pacman.conf");
    fs::write(&config,format!("[options]\nRootDir = {}\nDBPath = {}\nArchitecture = x86_64\nSigLevel = Never\nLogFile = {}\nHoldPkg = pacman glibc\nParallelDownloads = 3\nCheckSpace\n[extra]\nServer = https://example.org/$repo/$arch\n",root.display(),db.display(),home.join("pacman.log").display())).unwrap();
    let cfg = AlpmConfig::load(&config, &CommandRunner::default()).unwrap();
    assert!(cfg.checkspace);
    assert_eq!(cfg.parallel_downloads, 3);
    assert_eq!(cfg.holdpkgs, vec!["pacman", "glibc"]);
    (
        AlpmTransaction::preview(cfg, db.clone(), policy, "fixture-op".into()).unwrap(),
        plan,
        db,
        local,
    )
}
#[test]
fn native_preview_resolves_fixture_packages_but_cannot_commit() {
    use omarchy_app_updates::helper::SilentSink;
    let d = tempfile::tempdir().unwrap();
    let (mut native, plan, db, local) = native_fixture(d.path(), false);
    let actual = native.prepare(&plan).unwrap();
    assert!(
        actual.steps[0]
            .package_changes
            .iter()
            .any(|c| c.name == "ghostty" && c.new_version == "2.0-1")
    );
    assert!(
        actual.steps[0]
            .package_changes
            .iter()
            .any(|c| c.name == "fixture-lib" && c.action == PackageAction::Install)
    );
    assert_eq!(
        std::fs::read(db.join("local/ghostty-1.0-1/desc")).unwrap(),
        local
    );
    assert_eq!(
        native.commit(&actual.digest, &SilentSink).unwrap_err().code,
        ErrorCode::AuthUnavailable
    );
    native.release().unwrap();
    assert_eq!(
        std::fs::read(db.join("local/ghostty-1.0-1/desc")).unwrap(),
        local
    );
}
#[test]
fn native_provider_callback_waits_for_matching_explicit_choice() {
    let d = tempfile::tempdir().unwrap();
    let (mut native, plan, _, _) = native_fixture(d.path(), true);
    assert_eq!(native.prepare(&plan).unwrap_err().code, ErrorCode::Busy);
    let question = native.pending_question().unwrap();
    assert_eq!(question.kind, "provider");
    assert_eq!(question.choices.len(), 2);
    assert!(
        native
            .answer(
                &question.id,
                &DecisionValue::SelectProvider("unlisted".into())
            )
            .is_err()
    );
    native
        .answer(
            &question.id,
            &DecisionValue::SelectProvider("extra/impl-two=2.0-1".into()),
        )
        .unwrap();
    let prepared = native.prepare(&plan).unwrap();
    assert!(
        prepared.steps[0]
            .package_changes
            .iter()
            .any(|p| p.name == "impl-two")
    );
    assert!(
        !prepared.steps[0]
            .package_changes
            .iter()
            .any(|p| p.name == "impl-one")
    );
    assert_eq!(prepared.decisions.len(), 1);
    native.release().unwrap();
}
