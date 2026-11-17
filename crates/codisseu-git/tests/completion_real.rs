use codisseu_git::{rebase, sync, *};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn cli(root: &Path, a: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(a)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{a:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().into()
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    cli(d.path(), &["init", "-b", "main"]);
    cli(d.path(), &["config", "user.name", "Completion Fixture"]);
    cli(
        d.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    cli(d.path(), &["config", "commit.gpgsign", "false"]);
    let hooks = d.path().join("hooks");
    fs::create_dir(&hooks).unwrap();
    cli(
        d.path(),
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    d
}
fn commit(root: &Path, name: &str, text: &str, message: &str) -> String {
    fs::write(root.join(name), text).unwrap();
    cli(root, &["add", "--", name]);
    cli(root, &["commit", "-m", message]);
    cli(root, &["rev-parse", "HEAD"])
}
fn settle(s: &mut GitService) {
    let t = Instant::now();
    while s.is_busy() {
        s.poll();
        assert!(t.elapsed() < Duration::from_secs(20), "{:?}", s.error);
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
}
fn send(s: &mut GitService, c: GitCommand) {
    s.dispatch(c).unwrap();
    settle(s);
    assert!(s.error.is_none(), "{:?}", s.error);
}
fn apply(s: &mut GitService) {
    let p = s.preview.clone().unwrap();
    send(
        s,
        GitCommand::Apply {
            preview_id: p.id,
            repo_generation: p.generation,
        },
    );
    assert!(s.report.as_ref().unwrap().success);
}
fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codisseu-git-helper"))
}
#[test]
fn push_source_and_destination_differ_and_review_is_sealed_by_refs_and_trust() {
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let remote = tempfile::tempdir().unwrap();
    cli(remote.path(), &["init", "--bare", "-b", "main"]);
    cli(
        d.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    cli(d.path(), &["push", "origin", "main"]);
    cli(d.path(), &["checkout", "-b", "source"]);
    let tip = commit(d.path(), "feature", "feature\n", "feature");
    cli(d.path(), &["checkout", "main"]);
    let mut s = GitService::new();
    send(
        &mut s,
        GitCommand::Open {
            path: d.path().into(),
            trusted: false,
        },
    );
    assert!(
        s.dispatch(GitCommand::PushReview {
            remote: "origin".into(),
            source: "source".into(),
            target: "release".into()
        })
        .is_err()
    );
    send(&mut s, GitCommand::Trust(true));
    send(
        &mut s,
        GitCommand::PushReview {
            remote: "origin".into(),
            source: "source".into(),
            target: "release".into(),
        },
    );
    let review = s.push_review.as_ref().unwrap();
    assert_eq!(review.local_oid, tip);
    assert!(review.remote_oid.is_none());
    assert!(
        review
            .changes
            .iter()
            .any(|c| c.path == Path::new("feature"))
    );
    send(&mut s, GitCommand::Trust(true));
    send(
        &mut s,
        GitCommand::WorkflowPrepare(WorkflowAction::PushTo {
            remote: "origin".into(),
            source: "source".into(),
            target: "release".into(),
            tags: false,
        }),
    );
    apply(&mut s);
    assert_eq!(
        cli(remote.path(), &["rev-parse", "refs/heads/release"]),
        tip
    );
    assert_eq!(cli(remote.path(), &["rev-parse", "refs/heads/main"]), base);
    assert_eq!(cli(d.path(), &["rev-parse", "HEAD"]), base);
    send(
        &mut s,
        GitCommand::PushReview {
            remote: "origin".into(),
            source: "source".into(),
            target: "release".into(),
        },
    );
    commit(d.path(), "other", "changed\n", "external change");
    s.dispatch(GitCommand::WorkflowPrepare(WorkflowAction::PushTo {
        remote: "origin".into(),
        source: "source".into(),
        target: "release".into(),
        tags: false,
    }))
    .unwrap();
    settle(&mut s);
    assert!(s.error.is_some());
    assert_eq!(
        cli(remote.path(), &["rev-parse", "refs/heads/release"]),
        tip
    );
}
#[test]
fn interactive_rebase_reorders_rewords_squashes_fixes_and_drops_real_commits() {
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let a = commit(d.path(), "a", "a\n", "A\n\noriginal body");
    let b = commit(d.path(), "b", "b\n", "B");
    let c = commit(d.path(), "c", "c\n", "C");
    let e = commit(d.path(), "drop", "drop\n", "Drop");
    let f = commit(d.path(), "f", "f\n", "Fixup");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    assert_eq!(read.series().commits[0].message, "A\n\noriginal body\n");
    let step = |oid: String, action, message: Option<&str>| RebaseStep {
        oid,
        action,
        message: message.map(str::to_owned),
    };
    let plan = rebase::prepare_rebase(
        &repo,
        &read,
        &[
            step(
                b,
                RebaseAction::Reword,
                Some("B revised\n\nbody retained deliberately\n"),
            ),
            step(a, RebaseAction::Pick, None),
            step(c, RebaseAction::Squash, Some("A plus C\n\ncombined body\n")),
            step(e, RebaseAction::Drop, None),
            step(f, RebaseAction::Fixup, None),
        ],
        &AtomicBool::new(false),
    )
    .unwrap();
    let result =
        rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).unwrap();
    assert!(result.success, "{:?}", result);
    assert!(result.operation.is_none());
    let log = cli(d.path(), &["log", "--format=%s", &format!("{base}..HEAD")]);
    assert_eq!(log, "A plus C\nB revised");
    assert_eq!(
        cli(d.path(), &["log", "-1", "--format=%B"]),
        "A plus C\n\ncombined body"
    );
    assert_eq!(
        cli(d.path(), &["log", "-1", "--format=%B", "HEAD~1"]),
        "B revised\n\nbody retained deliberately"
    );
    assert!(!d.path().join("drop").exists());
    for file in ["a", "b", "c", "f"] {
        assert!(d.path().join(file).exists());
    }
}
#[test]
fn interactive_edit_stops_continue_and_abort_restore_real_state() {
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let a = commit(d.path(), "a", "a\n", "A");
    let b = commit(d.path(), "b", "b\n", "B");
    let original = cli(d.path(), &["rev-parse", "HEAD"]);
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let steps = vec![
        RebaseStep {
            oid: a,
            action: RebaseAction::Edit,
            message: None,
        },
        RebaseStep {
            oid: b,
            action: RebaseAction::Pick,
            message: None,
        },
    ];
    let plan = rebase::prepare_rebase(&repo, &read, &steps, &AtomicBool::new(false)).unwrap();
    let report =
        rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).unwrap();
    assert_eq!(report.operation, Some(OperationKind::Rebase));
    assert!(d.path().join("a").exists());
    assert!(!d.path().join("b").exists());
    fs::write(d.path().join("a"), "A edited at pause\n").unwrap();
    cli(d.path(), &["add", "a"]);
    let done =
        rebase::continue_rebase_with_helper(&repo, false, &helper(), &AtomicBool::new(false))
            .unwrap();
    assert!(done.success, "{:?}", done);
    assert!(done.operation.is_none());
    assert_eq!(
        fs::read(d.path().join("a")).unwrap(),
        b"A edited at pause\n"
    );
    let reread = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let steps = reread
        .series()
        .commits
        .iter()
        .enumerate()
        .map(|(i, c)| RebaseStep {
            oid: c.commit.oid.clone(),
            action: if i == 0 {
                RebaseAction::Edit
            } else {
                RebaseAction::Pick
            },
            message: None,
        })
        .collect::<Vec<_>>();
    let plan = rebase::prepare_rebase(&repo, &reread, &steps, &AtomicBool::new(false)).unwrap();
    let before = cli(d.path(), &["rev-parse", "HEAD"]);
    rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).unwrap();
    let aborted =
        rebase::continue_rebase_with_helper(&repo, true, &helper(), &AtomicBool::new(false))
            .unwrap();
    assert!(aborted.success, "{:?}", aborted);
    assert_eq!(cli(d.path(), &["rev-parse", "HEAD"]), before);
    assert!(d.path().join("b").exists());
    assert!(!original.is_empty());
}
#[test]
fn push_modes_limit_current_tags_and_all_adds_lightweight_and_other_branch_tags() {
    let d = fixture();
    commit(d.path(), "base", "base\n", "base");
    let remote = tempfile::tempdir().unwrap();
    cli(remote.path(), &["init", "--bare", "-b", "main"]);
    cli(
        d.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    cli(d.path(), &["push", "origin", "main"]);
    cli(d.path(), &["checkout", "-b", "source"]);
    commit(d.path(), "source", "source\n", "source");
    cli(d.path(), &["tag", "-a", "reachable", "-m", "reachable"]);
    cli(d.path(), &["tag", "lightweight"]);
    cli(d.path(), &["checkout", "-b", "other", "main"]);
    commit(d.path(), "other", "other\n", "other");
    cli(
        d.path(),
        &["tag", "-a", "unreachable", "-m", "other branch"],
    );
    cli(d.path(), &["checkout", "source"]);
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read =
        sync::inspect_push(&repo, "origin", "source", "dest", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        read.review()
            .tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        vec!["reachable"]
    );
    assert_eq!(read.review().all_tags.len(), 3);
    let plan = sync::prepare_push_mode(
        &repo,
        &read,
        PushTagsMode::CurrentBranch,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        sync::apply_push(&repo, &plan, &AtomicBool::new(false))
            .unwrap()
            .success
    );
    assert_eq!(cli(remote.path(), &["tag", "--list"]), "reachable");
    let read =
        sync::inspect_push(&repo, "origin", "source", "dest", &AtomicBool::new(false)).unwrap();
    let plan =
        sync::prepare_push_mode(&repo, &read, PushTagsMode::All, &AtomicBool::new(false)).unwrap();
    assert!(
        sync::apply_push(&repo, &plan, &AtomicBool::new(false))
            .unwrap()
            .success
    );
    assert_eq!(
        cli(remote.path(), &["tag", "--list"]),
        "lightweight\nreachable\nunreachable"
    );
    let other = cli(d.path(), &["rev-parse", "other"]);
    cli(
        remote.path(),
        &["update-ref", "refs/tags/lightweight", &other],
    );
    let read =
        sync::inspect_push(&repo, "origin", "source", "dest", &AtomicBool::new(false)).unwrap();
    assert!(
        sync::prepare_push_mode(&repo, &read, PushTagsMode::All, &AtomicBool::new(false)).is_err()
    );
}
#[test]
fn remote_advance_invalidates_review_and_non_fast_forward_is_never_forced() {
    let d = fixture();
    commit(d.path(), "base", "base\n", "base");
    let remote = tempfile::tempdir().unwrap();
    cli(remote.path(), &["init", "--bare", "-b", "main"]);
    cli(
        d.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    cli(d.path(), &["push", "origin", "main"]);
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read =
        sync::inspect_push(&repo, "origin", "main", "main", &AtomicBool::new(false)).unwrap();
    let peer = tempfile::tempdir().unwrap();
    cli(
        peer.path(),
        &["clone", remote.path().to_str().unwrap(), "peer"],
    );
    let peer = peer.path().join("peer");
    cli(&peer, &["config", "user.name", "Peer"]);
    cli(&peer, &["config", "user.email", "peer@example.invalid"]);
    cli(&peer, &["config", "commit.gpgsign", "false"]);
    let new = commit(&peer, "remote", "remote\n", "remote advance");
    cli(&peer, &["push", "origin", "main"]);
    assert!(sync::prepare_push(&repo, &read, false, &AtomicBool::new(false)).is_err());
    let missing =
        sync::inspect_push(&repo, "origin", "main", "main", &AtomicBool::new(false)).unwrap_err();
    assert!(missing.to_string().contains("Fetch"));
    cli(d.path(), &["fetch", "origin"]);
    let read =
        sync::inspect_push(&repo, "origin", "main", "main", &AtomicBool::new(false)).unwrap();
    assert!(!read.review().can_push);
    assert!(sync::prepare_push(&repo, &read, false, &AtomicBool::new(false)).is_err());
    assert_eq!(cli(remote.path(), &["rev-parse", "main"]), new);
}
#[test]
fn rebase_conflict_continue_uses_planned_full_message_after_user_resolution() {
    let d = fixture();
    let base = commit(d.path(), "x", "base\n", "base");
    let a = commit(d.path(), "x", "A\n", "A");
    let b = commit(d.path(), "x", "B\n", "B");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let steps = vec![
        RebaseStep {
            oid: b,
            action: RebaseAction::Reword,
            message: Some("reviewed message\n\nfull body\n".into()),
        },
        RebaseStep {
            oid: a,
            action: RebaseAction::Drop,
            message: None,
        },
    ];
    let plan = rebase::prepare_rebase(&repo, &read, &steps, &AtomicBool::new(false)).unwrap();
    let result =
        rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).unwrap();
    assert!(!result.success);
    assert_eq!(result.operation, Some(OperationKind::Rebase));
    assert_eq!(result.conflicts, vec![PathBuf::from("x")]);
    fs::write(d.path().join("x"), "user resolved\n").unwrap();
    cli(d.path(), &["add", "x"]);
    let done =
        rebase::continue_rebase_with_helper(&repo, false, &helper(), &AtomicBool::new(false))
            .unwrap();
    assert!(done.success, "{:?}", done);
    assert!(done.operation.is_none());
    assert_eq!(
        cli(d.path(), &["log", "-1", "--format=%B"]),
        "reviewed message\n\nfull body"
    );
    assert_eq!(fs::read(d.path().join("x")).unwrap(), b"user resolved\n");
}
#[test]
fn rebase_rejects_stale_duplicate_omitted_and_leading_squash_without_mutating() {
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let a = commit(d.path(), "a", "a\n", "A");
    let b = commit(d.path(), "b", "b\n", "B");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let pick = |oid: String| RebaseStep {
        oid,
        action: RebaseAction::Pick,
        message: None,
    };
    assert!(
        rebase::prepare_rebase(
            &repo,
            &read,
            &[pick(a.clone()), pick(a.clone())],
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        rebase::prepare_rebase(&repo, &read, &[pick(a.clone())], &AtomicBool::new(false)).is_err()
    );
    assert!(
        rebase::prepare_rebase(
            &repo,
            &read,
            &[
                RebaseStep {
                    oid: a.clone(),
                    action: RebaseAction::Squash,
                    message: None
                },
                pick(b.clone())
            ],
            &AtomicBool::new(false)
        )
        .is_err()
    );
    let plan = rebase::prepare_rebase(
        &repo,
        &read,
        &[pick(a), pick(b.clone())],
        &AtomicBool::new(false),
    )
    .unwrap();
    fs::write(d.path().join("a"), "externally changed\n").unwrap();
    assert!(
        rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).is_err()
    );
    assert_eq!(cli(d.path(), &["rev-parse", "HEAD"]), b);
    assert_eq!(
        fs::read(d.path().join("a")).unwrap(),
        b"externally changed\n"
    );
    assert!(!d.path().join(".git/rebase-merge").exists());
}
#[test]
fn rebase_cancel_interrupts_running_hook_and_reports_unknown_without_rollback_claim() {
    use std::sync::{Arc, atomic::Ordering};
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let a = commit(d.path(), "a", "a\n", "A");
    let hooks = d.path().join(".git/owned-hooks");
    fs::create_dir(&hooks).unwrap();
    cli(
        d.path(),
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    let hook = hooks.join("pre-rebase");
    fs::write(
        &hook,
        "#!/bin/sh\nprintf started > .git/rebase-hook-started\nsleep 30\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let plan = rebase::prepare_rebase(
        &repo,
        &read,
        &[RebaseStep {
            oid: a.clone(),
            action: RebaseAction::Pick,
            message: None,
        }],
        &AtomicBool::new(false),
    )
    .unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let marker = d.path().join(".git/rebase-hook-started");
    let started = Instant::now();
    let thread = std::thread::spawn(move || {
        while !marker.exists() && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        flag.store(true, Ordering::SeqCst);
    });
    let report = rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &cancel).unwrap();
    thread.join().unwrap();
    assert!(report.outcome_unknown);
    assert!(!report.success);
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(cli(d.path(), &["rev-parse", "HEAD"]), a);
    assert!(!report.summary.contains("revertid"));
}
#[test]
fn stash_details_includes_untracked_and_keep_index_preserves_staging() {
    let d = fixture();
    commit(d.path(), "tracked", "base\n", "base");
    fs::write(d.path().join("tracked"), "staged\n").unwrap();
    cli(d.path(), &["add", "tracked"]);
    fs::write(d.path().join("tracked"), "working\n").unwrap();
    fs::write(d.path().join("untracked"), "untracked\n").unwrap();
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: d.path().into(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::CommitDetails {
            commit: "HEAD".into(),
        },
    );
    let log_oid = service.details.as_ref().unwrap().commit.clone();
    send(
        &mut service,
        GitCommand::WorkflowPrepare(WorkflowAction::StashCreateOptions {
            message: "keep index".into(),
            include_untracked: true,
            keep_index: true,
        }),
    );
    apply(&mut service);
    assert_eq!(fs::read(d.path().join("tracked")).unwrap(), b"staged\n");
    assert_eq!(
        cli(d.path(), &["diff", "--cached", "--name-only"]),
        "tracked"
    );
    assert!(!d.path().join("untracked").exists());
    send(&mut service, GitCommand::Stashes);
    let reference = service.stashes[0].reference.clone();
    send(&mut service, GitCommand::StashDetails { stash: reference });
    let detail = service.stash_details.as_ref().unwrap();
    assert!(
        detail
            .changes
            .iter()
            .any(|c| c.path == Path::new("tracked"))
    );
    assert!(
        detail
            .changes
            .iter()
            .any(|c| c.path == Path::new("untracked"))
    );
    assert!(String::from_utf8_lossy(&detail.patch).contains("+untracked"));
    assert_eq!(service.details.as_ref().unwrap().commit, log_oid);
}
#[test]
fn continue_approval_detects_changed_todo_and_corrupt_editor_state_still_allows_abort() {
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let a = commit(d.path(), "a", "a\n", "A");
    let b = commit(d.path(), "b", "b\n", "B");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let plan = rebase::prepare_rebase(
        &repo,
        &read,
        &[
            RebaseStep {
                oid: a,
                action: RebaseAction::Edit,
                message: None,
            },
            RebaseStep {
                oid: b.clone(),
                action: RebaseAction::Pick,
                message: None,
            },
        ],
        &AtomicBool::new(false),
    )
    .unwrap();
    rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).unwrap();
    let approval = workflows::prepare_workflow(
        &repo,
        WorkflowAction::Continue(OperationKind::Rebase),
        &AtomicBool::new(false),
    )
    .unwrap();
    let path = d.path().join(".git/rebase-merge/git-rebase-todo");
    let mut todo = fs::read_to_string(&path).unwrap();
    todo.push_str("\n# changed externally\n");
    fs::write(&path, todo).unwrap();
    assert!(workflows::apply_workflow(&repo, &approval, &AtomicBool::new(false)).is_err());
    fs::write(
        d.path().join(".git/codisseu/interactive-rebase.json"),
        "not valid JSON",
    )
    .unwrap();
    let aborted =
        rebase::continue_rebase_with_helper(&repo, true, &helper(), &AtomicBool::new(false))
            .unwrap();
    assert!(aborted.success, "{:?}", aborted);
    assert_eq!(cli(d.path(), &["rev-parse", "HEAD"]), b);
}
#[test]
fn atomic_push_rejection_keeps_branch_and_tags_unchanged() {
    let d = fixture();
    let base = commit(d.path(), "base", "base\n", "base");
    let remote = tempfile::tempdir().unwrap();
    cli(remote.path(), &["init", "--bare", "-b", "main"]);
    cli(
        d.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    cli(d.path(), &["push", "origin", "main"]);
    commit(d.path(), "new", "new\n", "new");
    cli(d.path(), &["tag", "-a", "rejected-tag", "-m", "tag"]);
    let hook = remote.path().join("hooks/pre-receive");
    fs::write(&hook,"#!/bin/sh\nwhile read old new name; do\ncase \"$name\" in refs/tags/*) exit 1;; esac\ndone\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let read =
        sync::inspect_push(&repo, "origin", "main", "main", &AtomicBool::new(false)).unwrap();
    let plan =
        sync::prepare_push_mode(&repo, &read, PushTagsMode::All, &AtomicBool::new(false)).unwrap();
    let report = sync::apply_push(&repo, &plan, &AtomicBool::new(false)).unwrap();
    assert!(!report.success);
    assert_eq!(cli(remote.path(), &["rev-parse", "main"]), base);
    assert!(cli(remote.path(), &["tag", "--list"]).is_empty());
}
#[test]
fn new_target_excludes_old_remote_history_and_revision_diff_includes_earlier_outgoing_file() {
    let d = fixture();
    let first = commit(d.path(), "base", "base\n", "base");
    let tree = cli(d.path(), &["rev-parse", "HEAD^{tree}"]);
    let mut parent = first;
    for i in 0..215 {
        parent = cli(
            d.path(),
            &[
                "commit-tree",
                &tree,
                "-p",
                &parent,
                "-m",
                &format!("already remote{i}"),
            ],
        );
    }
    cli(d.path(), &["update-ref", "refs/heads/main", &parent]);
    let remote = tempfile::tempdir().unwrap();
    cli(remote.path(), &["init", "--bare", "-b", "main"]);
    cli(
        d.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    cli(d.path(), &["push", "origin", "main"]);
    cli(d.path(), &["checkout", "-b", "feature"]);
    commit(d.path(), "earlier", "earlier content\n", "earlier outgoing");
    let head = commit(d.path(), "later", "later content\n", "last outgoing");
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: d.path().into(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::PushReview {
            remote: "origin".into(),
            source: "feature".into(),
            target: "new-target".into(),
        },
    );
    let read = service.push_review.clone().unwrap();
    assert_eq!(read.commits.len(), 2);
    assert_eq!(read.diff_base.as_deref(), Some(parent.as_str()));
    assert!(read.changes.iter().any(|c| c.path == Path::new("earlier")));
    assert!(read.changes.iter().any(|c| c.path == Path::new("later")));
    assert!(!read.changes.iter().any(|c| c.path == Path::new("base")));
    send(
        &mut service,
        GitCommand::CommitDetails {
            commit: head.clone(),
        },
    );
    assert!(
        !String::from_utf8_lossy(&service.details.as_ref().unwrap().patch)
            .contains("+earlier content")
    );
    send(
        &mut service,
        GitCommand::RevisionDiff {
            base: read.diff_base.clone(),
            head: head.clone(),
            path: Some("earlier".into()),
        },
    );
    let diff = service.revision_diff.as_ref().unwrap();
    assert_eq!(diff.base, read.diff_base);
    assert_eq!(diff.head, head);
    assert!(String::from_utf8_lossy(&diff.bytes).contains("+earlier content"));
    assert!(!String::from_utf8_lossy(&diff.bytes).contains("+later content"));
    assert_eq!(service.details.as_ref().unwrap().commit, head);
    send(
        &mut service,
        GitCommand::WorkflowPrepare(WorkflowAction::PushTo {
            remote: "origin".into(),
            source: "feature".into(),
            target: "new-target".into(),
            tags: false,
        }),
    );
    apply(&mut service);
    assert_eq!(cli(remote.path(), &["rev-parse", "new-target"]), head);
}
#[test]
fn rust_editor_handles_repository_spaces_and_keeps_message_as_literal_text() {
    let fixture = fixture();
    let d = fixture.path().join("repo é com espaços");
    fs::create_dir(&d).unwrap();
    cli(&d, &["init", "-b", "main"]);
    cli(&d, &["config", "user.name", "Fixture"]);
    cli(&d, &["config", "user.email", "fixture@example.invalid"]);
    cli(&d, &["config", "commit.gpgsign", "false"]);
    let base = commit(&d, "base", "base\n", "base");
    let oid = commit(&d, "a", "a\n", "A");
    let repo = Repository::discover(&d, &AtomicBool::new(false)).unwrap();
    let read = rebase::inspect_rebase(&repo, &base, &AtomicBool::new(false)).unwrap();
    let message =
        "literal message\n\n# literal heading\n$(touch owned-marker)\n`touch another-marker`\n";
    let plan = rebase::prepare_rebase(
        &repo,
        &read,
        &[RebaseStep {
            oid,
            action: RebaseAction::Reword,
            message: Some(message.into()),
        }],
        &AtomicBool::new(false),
    )
    .unwrap();
    let report =
        rebase::apply_rebase_with_helper(&repo, &plan, &helper(), &AtomicBool::new(false)).unwrap();
    assert!(report.success, "{:?}", report);
    assert_eq!(cli(&d, &["log", "-1", "--format=%B"]), message.trim_end());
    assert!(!d.join("owned-marker").exists());
    assert!(!d.join("another-marker").exists());
}
#[test]
fn bound_hunk_preview_rejects_external_ordinal_shift_and_stages_exact_visible_patch() {
    let d = fixture();
    let original = (0..100).map(|i| format!("line{i}\n")).collect::<String>();
    commit(d.path(), "a.txt", &original, "base");
    let edited = original
        .replace("line30\n", "changed30\n")
        .replace("line80\n", "changed80\n");
    fs::write(d.path().join("a.txt"), &edited).unwrap();
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: d.path().into(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::Diff {
            path: "a.txt".into(),
            side: DiffSide::Working,
        },
    );
    let visible = service.diff.as_ref().unwrap().bytes.clone();
    assert_eq!(service.diff.as_ref().unwrap().hunks.len(), 2);
    fs::write(
        d.path().join("a.txt"),
        edited.replace("line2\n", "external2\n"),
    )
    .unwrap();
    service
        .dispatch(GitCommand::HunksPrepare {
            path: "a.txt".into(),
            hunks: vec![0],
            unstage: false,
            expected_patch: visible.clone(),
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_some());
    assert!(service.preview.is_none());
    assert!(cli(d.path(), &["diff", "--cached"]).is_empty());
    service
        .dispatch(GitCommand::Prepare(LocalAction::StageHunks {
            path: "a.txt".into(),
            hunks: vec![0],
            unstage: false,
        }))
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_some());
    assert!(cli(d.path(), &["diff", "--cached"]).is_empty());
    fs::write(d.path().join("a.txt"), &edited).unwrap();
    send(
        &mut service,
        GitCommand::HunksPrepare {
            path: "a.txt".into(),
            hunks: vec![1],
            unstage: false,
            expected_patch: visible.clone(),
        },
    );
    apply(&mut service);
    let staged = cli(d.path(), &["show", ":a.txt"]);
    assert!(staged.contains("changed80"));
    assert!(!staged.contains("changed30"));
    service
        .dispatch(GitCommand::HunksPrepare {
            path: "a.txt".into(),
            hunks: vec![0],
            unstage: true,
            expected_patch: visible,
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_some());
    assert_eq!(cli(d.path(), &["show", ":a.txt"]), staged);
    assert!(
        service
            .dispatch(GitCommand::HunksPrepare {
                path: "a.txt".into(),
                hunks: vec![0],
                unstage: false,
                expected_patch: vec![b'x'; 8 * 1024 * 1024 + 1]
            })
            .is_err()
    );
}
#[test]
fn multiple_hunks_with_at_signs_in_filename_apply_one_valid_patch() {
    let d = fixture();
    let path = "name @@ sample.txt";
    let original = (0..70).map(|i| format!("line{i}\n")).collect::<String>();
    commit(d.path(), path, &original, "base");
    fs::write(
        d.path().join(path),
        original
            .replace("line2\n", "edit2\n")
            .replace("line60\n", "edit60\n"),
    )
    .unwrap();
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: d.path().into(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::Diff {
            path: path.into(),
            side: DiffSide::Working,
        },
    );
    let visible = service.diff.as_ref().unwrap().bytes.clone();
    assert_eq!(service.diff.as_ref().unwrap().hunks.len(), 2);
    send(
        &mut service,
        GitCommand::HunksPrepare {
            path: path.into(),
            hunks: vec![0, 1],
            unstage: false,
            expected_patch: visible,
        },
    );
    apply(&mut service);
    let staged = cli(d.path(), &["show", &format!(":{path}")]);
    assert!(staged.contains("edit2"));
    assert!(staged.contains("edit60"));
    assert!(cli(d.path(), &["diff", "--", path]).is_empty());
}
