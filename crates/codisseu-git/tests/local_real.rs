use codisseu_git::*;
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().into()
}
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-b", "main"]);
    git(d.path(), &["config", "user.name", "Fixture"]);
    git(
        d.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    git(d.path(), &["config", "core.autocrlf", "false"]);
    let hooks = d.path().join("empty-hooks");
    fs::create_dir(&hooks).unwrap();
    git(
        d.path(),
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    d
}
fn settle(s: &mut GitService) {
    let start = Instant::now();
    while s.is_busy() {
        s.poll();
        assert!(start.elapsed() < Duration::from_secs(20), "{:?}", s.error);
        std::thread::sleep(Duration::from_millis(5));
    }
    s.poll();
}
fn send(s: &mut GitService, c: GitCommand) {
    s.dispatch(c).unwrap();
    settle(s);
    assert!(s.error.is_none(), "{:?}", s.error);
}
fn open(d: &Path) -> GitService {
    let mut s = GitService::new();
    send(
        &mut s,
        GitCommand::Open {
            path: d.into(),
            trusted: true,
        },
    );
    s
}
fn apply(s: &mut GitService, a: LocalAction) {
    send(s, GitCommand::Prepare(a));
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
fn base(d: &Path) {
    fs::write(d.join("a.txt"), "base\n").unwrap();
    fs::write(d.join("b.txt"), "base\n").unwrap();
    git(d, &["add", "--all"]);
    git(d, &["commit", "-m", "base"]);
}
#[test]
fn branch_log_excludes_stash_snapshots_but_stash_remains_readable() {
    let d = fixture();
    base(d.path());
    let original = git(d.path(), &["rev-parse", "HEAD"]);
    fs::write(d.path().join("a.txt"), "stored change\n").unwrap();
    git(d.path(), &["stash", "push", "-m", "separate stash"]);
    let mut service = open(d.path());
    send(
        &mut service,
        GitCommand::History {
            skip: 0,
            limit: 200,
            path: None,
        },
    );
    let page = service.history.as_ref().unwrap();
    assert_eq!(page.commits.len(), 1);
    assert_eq!(page.commits[0].oid, original);
    send(&mut service, GitCommand::Stashes);
    assert_eq!(service.stashes.len(), 1);
    send(
        &mut service,
        GitCommand::StashDetails {
            stash: "stash@{0}".into(),
        },
    );
    assert!(
        String::from_utf8_lossy(&service.stash_details.as_ref().unwrap().patch)
            .contains("stored change")
    );
}
#[test]
fn unborn_stage_unstage_and_selected_working_commit() {
    let d = fixture();
    fs::write(d.path().join("-á\n.js"), "let x=1;\r\n").unwrap();
    fs::write(d.path().join("other"), "excluded\n").unwrap();
    let mut s = open(d.path());
    assert!(s.snapshot.as_ref().unwrap().head.is_none());
    apply(
        &mut s,
        LocalAction::Stage {
            paths: vec!["other".into()],
        },
    );
    apply(
        &mut s,
        LocalAction::Unstage {
            paths: vec!["other".into()],
        },
    );
    assert!(git(d.path(), &["ls-files"]).is_empty());
    apply(
        &mut s,
        LocalAction::Stage {
            paths: vec!["other".into()],
        },
    );
    apply(
        &mut s,
        LocalAction::Commit {
            selection: CommitSelection::WorkingPaths(vec!["-á\n.js".into()]),
            message: "selected first".into(),
            amend: false,
        },
    );
    let tree = git(d.path(), &["ls-tree", "--name-only", "HEAD"]);
    assert!(tree.contains("js"));
    assert!(!tree.contains("other"));
    assert!(git(d.path(), &["diff", "--cached", "--name-only"]).contains("other"));
    let r = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        r.file_at("HEAD", Path::new("-á\n.js"), &AtomicBool::new(false))
            .unwrap()
            .bytes,
        b"let x=1;\r\n"
    );
}
#[test]
fn index_only_commit_preserves_unstaged_and_excluded_index() {
    let d = fixture();
    base(d.path());
    fs::write(d.path().join("a.txt"), "staged\n").unwrap();
    fs::write(d.path().join("b.txt"), "excluded\n").unwrap();
    git(d.path(), &["add", "--all"]);
    fs::write(d.path().join("a.txt"), "working\n").unwrap();
    let mut s = open(d.path());
    apply(
        &mut s,
        LocalAction::Commit {
            selection: CommitSelection::Paths(vec!["a.txt".into()]),
            message: "index only".into(),
            amend: false,
        },
    );
    assert_eq!(git(d.path(), &["show", "HEAD:a.txt"]), "staged");
    assert_eq!(git(d.path(), &["show", "HEAD:b.txt"]), "base");
    assert_eq!(
        fs::read_to_string(d.path().join("a.txt")).unwrap(),
        "working\n"
    );
    assert_eq!(git(d.path(), &["diff", "--cached", "--name-only"]), "b.txt");
}
#[test]
fn stale_preview_does_not_stage_changed_bytes() {
    let d = fixture();
    base(d.path());
    fs::write(d.path().join("a.txt"), "first\n").unwrap();
    let mut s = open(d.path());
    send(
        &mut s,
        GitCommand::Prepare(LocalAction::Stage {
            paths: vec!["a.txt".into()],
        }),
    );
    let p = s.preview.clone().unwrap();
    fs::write(d.path().join("a.txt"), "other\n").unwrap();
    s.dispatch(GitCommand::Apply {
        preview_id: p.id,
        repo_generation: p.generation,
    })
    .unwrap();
    settle(&mut s);
    assert!(!s.report.as_ref().unwrap().success);
    assert!(!s.report.as_ref().unwrap().outcome_unknown);
    assert!(git(d.path(), &["diff", "--cached", "--name-only"]).is_empty());
}
#[test]
fn branches_tags_history_blame_details_worktree_and_rename() {
    let d = fixture();
    base(d.path());
    let mut s = open(d.path());
    apply(
        &mut s,
        LocalAction::CreateBranch {
            name: "feature/test".into(),
            start: None,
            checkout: true,
        },
    );
    assert_eq!(git(d.path(), &["branch", "--show-current"]), "feature/test");
    git(d.path(), &["mv", "a.txt", "renamed á.txt"]);
    send(&mut s, GitCommand::Refresh);
    assert_eq!(
        s.snapshot.as_ref().unwrap().entries[0].original_path,
        Some("a.txt".into())
    );
    apply(
        &mut s,
        LocalAction::Commit {
            selection: CommitSelection::AllStaged,
            message: "rename".into(),
            amend: false,
        },
    );
    apply(
        &mut s,
        LocalAction::CreateTag {
            name: "v1".into(),
            target: "HEAD".into(),
            message: Some("owned annotation".into()),
        },
    );
    send(&mut s, GitCommand::Tags);
    assert!(s.tags[0].annotated);
    send(&mut s, GitCommand::Branches);
    assert!(
        s.branches
            .iter()
            .any(|b| b.current && b.name == "feature/test")
    );
    send(
        &mut s,
        GitCommand::History {
            skip: 0,
            limit: 1,
            path: None,
        },
    );
    assert_eq!(s.history.as_ref().unwrap().commits[0].subject, "rename");
    assert!(s.history.as_ref().unwrap().has_more);
    send(
        &mut s,
        GitCommand::CommitDetails {
            commit: "HEAD".into(),
        },
    );
    assert_eq!(s.details.as_ref().unwrap().changes[0].status, 'R');
    send(
        &mut s,
        GitCommand::Blame {
            path: "renamed á.txt".into(),
            start: 1,
            lines: 500,
        },
    );
    assert_eq!(s.blame[0].text, b"base");
    let wd = tempfile::tempdir().unwrap();
    let linked = wd.path().join("linked");
    git(
        d.path(),
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let repo = Repository::discover(&linked, &AtomicBool::new(false)).unwrap();
    assert!(repo.info().linked_worktree);
    assert_ne!(repo.info().git_dir, repo.info().common_dir);
    assert!(
        repo.snapshot(&AtomicBool::new(false))
            .unwrap()
            .entries
            .is_empty()
    );
}
#[test]
fn hooks_failure_preserves_real_index_and_head() {
    let d = fixture();
    base(d.path());
    fs::write(d.path().join("a.txt"), "edited\n").unwrap();
    let hook = d.path().join("empty-hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nexit 7\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let head = git(d.path(), &["rev-parse", "HEAD"]);
    let before = fs::read(d.path().join(".git/index")).unwrap();
    let mut s = open(d.path());
    send(
        &mut s,
        GitCommand::Prepare(LocalAction::Commit {
            selection: CommitSelection::WorkingPaths(vec!["a.txt".into()]),
            message: "failing".into(),
            amend: false,
        }),
    );
    let p = s.preview.clone().unwrap();
    s.dispatch(GitCommand::Apply {
        preview_id: p.id,
        repo_generation: p.generation,
    })
    .unwrap();
    settle(&mut s);
    assert!(!s.report.as_ref().unwrap().success);
    assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
    assert!(!d.path().join(".git/index.lock").exists());
}
#[test]
fn selected_hunks_stage_and_reverse_and_discard() {
    let d = fixture();
    let original = (0..30).map(|i| format!("line{i}\n")).collect::<String>();
    fs::write(d.path().join("a.txt"), &original).unwrap();
    git(d.path(), &["add", "--all"]);
    git(d.path(), &["commit", "-m", "base"]);
    let changed = original
        .replace("line1\n", "changed1\n")
        .replace("line25\n", "changed25\n");
    fs::write(d.path().join("a.txt"), changed).unwrap();
    let mut s = open(d.path());
    send(
        &mut s,
        GitCommand::Diff {
            path: "a.txt".into(),
            side: DiffSide::Working,
        },
    );
    assert_eq!(s.diff.as_ref().unwrap().hunks.len(), 2);
    apply(
        &mut s,
        LocalAction::StageHunks {
            path: "a.txt".into(),
            hunks: vec![0],
            unstage: false,
        },
    );
    let staged = git(d.path(), &["show", ":a.txt"]);
    assert!(staged.contains("changed1"));
    assert!(!staged.contains("changed25"));
    send(
        &mut s,
        GitCommand::Diff {
            path: "a.txt".into(),
            side: DiffSide::Index,
        },
    );
    apply(
        &mut s,
        LocalAction::StageHunks {
            path: "a.txt".into(),
            hunks: vec![0],
            unstage: true,
        },
    );
    assert!(git(d.path(), &["diff", "--cached"]).is_empty());
    apply(
        &mut s,
        LocalAction::Discard {
            paths: vec!["a.txt".into()],
        },
    );
    assert_eq!(
        fs::read_to_string(d.path().join("a.txt")).unwrap(),
        original
    );
}
#[test]
fn amend_keeps_unselected_prepared_and_full_message_available() {
    let d = fixture();
    base(d.path());
    let mut s = open(d.path());
    fs::write(d.path().join("b.txt"), "staged excluded\n").unwrap();
    git(d.path(), &["add", "b.txt"]);
    send(&mut s, GitCommand::Refresh);
    apply(
        &mut s,
        LocalAction::Commit {
            selection: CommitSelection::Paths(vec![]),
            message: "subject\n\nfull body\n".into(),
            amend: true,
        },
    );
    send(
        &mut s,
        GitCommand::CommitDetails {
            commit: "HEAD".into(),
        },
    );
    assert_eq!(
        s.details.as_ref().unwrap().message,
        "subject\n\nfull body\n"
    );
    assert_eq!(git(d.path(), &["show", "HEAD:b.txt"]), "base");
    assert_eq!(git(d.path(), &["diff", "--cached", "--name-only"]), "b.txt");
}
#[test]
fn configured_read_helpers_never_execute_on_open_or_diff() {
    let d = fixture();
    base(d.path());
    let marker = d.path().join("should-not-run");
    let hook = d.path().join("empty-hooks/helper");
    fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(
        d.path(),
        &["config", "core.fsmonitor", hook.to_str().unwrap()],
    );
    git(
        d.path(),
        &["config", "diff.fixture.command", hook.to_str().unwrap()],
    );
    git(
        d.path(),
        &["config", "diff.fixture.textconv", hook.to_str().unwrap()],
    );
    git(
        d.path(),
        &["config", "filter.fixture.clean", hook.to_str().unwrap()],
    );
    fs::write(
        d.path().join(".gitattributes"),
        "a.txt filter=fixture diff=fixture\n",
    )
    .unwrap();
    fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    let mut s = GitService::new();
    send(
        &mut s,
        GitCommand::Open {
            path: d.path().into(),
            trusted: false,
        },
    );
    send(
        &mut s,
        GitCommand::Diff {
            path: "a.txt".into(),
            side: DiffSide::Working,
        },
    );
    assert!(!marker.exists());
    assert!(
        s.dispatch(GitCommand::Prepare(LocalAction::Stage {
            paths: vec!["a.txt".into()]
        }))
        .is_err()
    );
}
#[test]
fn cancel_running_explicit_commit_terminates_group_and_preserves_index() {
    let d = fixture();
    base(d.path());
    fs::write(d.path().join("a.txt"), "edited\n").unwrap();
    let hook = d.path().join("empty-hooks/pre-commit");
    fs::write(
        &hook,
        "#!/bin/sh\nprintf started > hook-started\nsleep 30\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut s = open(d.path());
    send(
        &mut s,
        GitCommand::Prepare(LocalAction::Commit {
            selection: CommitSelection::WorkingPaths(vec!["a.txt".into()]),
            message: "cancel".into(),
            amend: false,
        }),
    );
    let p = s.preview.clone().unwrap();
    let head = git(d.path(), &["rev-parse", "HEAD"]);
    let before = fs::read(d.path().join(".git/index")).unwrap();
    s.dispatch(GitCommand::Apply {
        preview_id: p.id,
        repo_generation: p.generation,
    })
    .unwrap();
    let start = Instant::now();
    while !d.path().join("hook-started").exists() {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    s.dispatch(GitCommand::Cancel).unwrap();
    settle(&mut s);
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(s.report.as_ref().unwrap().outcome_unknown);
    assert_eq!(git(d.path(), &["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(d.path().join(".git/index")).unwrap(), before);
    assert!(!d.path().join(".git/index.lock").exists());
}
#[test]
fn clone_owned_local_fixture_and_output_limits() {
    let d = fixture();
    base(d.path());
    let target = tempfile::tempdir().unwrap();
    let destination = target.path().join("clone");
    let mut s = GitService::new();
    send(
        &mut s,
        GitCommand::Clone {
            url: d.path().to_string_lossy().into_owned(),
            destination: destination.clone(),
        },
    );
    assert_eq!(
        s.repo.as_ref().unwrap().root,
        destination.canonicalize().unwrap()
    );
    fs::write(d.path().join("big"), vec![b'x'; 9 * 1024 * 1024]).unwrap();
    git(d.path(), &["add", "big"]);
    git(d.path(), &["commit", "-m", "large fixture"]);
    let r = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let err = r
        .file_at("HEAD", Path::new("big"), &AtomicBool::new(false))
        .unwrap_err();
    assert_eq!(err.kind, GitErrorKind::OutputLimit);
}
#[test]
fn graph_pages_are_stable_and_untracked_diff_is_real() {
    let d = fixture();
    base(d.path());
    git(d.path(), &["checkout", "-b", "feature"]);
    fs::write(d.path().join("feature"), "feature\n").unwrap();
    git(d.path(), &["add", "feature"]);
    git(d.path(), &["commit", "-m", "feature"]);
    git(d.path(), &["checkout", "main"]);
    fs::write(d.path().join("main"), "main\n").unwrap();
    git(d.path(), &["add", "main"]);
    git(d.path(), &["commit", "-m", "main"]);
    git(d.path(), &["merge", "--no-ff", "feature", "-m", "merge"]);
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let all = repo.history(0, 200, None, &AtomicBool::new(false)).unwrap();
    let page = repo.history(2, 2, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(all.commits[0].parents.len(), 2);
    for (a, b) in all.commits[2..].iter().zip(&page.commits) {
        assert_eq!((&a.oid, a.lane, &a.edges), (&b.oid, b.lane, &b.edges));
    }
    fs::write(d.path().join("-new.js"), "const x=1;\n").unwrap();
    let diff = repo
        .diff(
            Path::new("-new.js"),
            DiffSide::Working,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&diff.bytes).contains("+const x=1;"));
    assert_eq!(diff.hunks.len(), 1);
}
#[cfg(unix)]
#[test]
fn non_unicode_paths_and_parent_symlinks_are_not_followed() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    let d = fixture();
    base(d.path());
    let name = std::ffi::OsStr::from_bytes(b"nonutf8-\xff.txt");
    fs::write(d.path().join(name), "data\n").unwrap();
    let mut s = open(d.path());
    assert!(
        s.snapshot
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path.as_os_str() == name)
    );
    apply(
        &mut s,
        LocalAction::Commit {
            selection: CommitSelection::WorkingPaths(vec![name.into()]),
            message: "raw path".into(),
            amend: false,
        },
    );
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        repo.file_at("HEAD", Path::new(name), &AtomicBool::new(false))
            .unwrap()
            .bytes,
        b"data\n"
    );
    fs::create_dir(d.path().join("nested")).unwrap();
    fs::write(d.path().join("nested/file"), "safe\n").unwrap();
    git(d.path(), &["add", "nested/file"]);
    git(d.path(), &["commit", "-m", "nested"]);
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "secret outside\n").unwrap();
    fs::remove_dir_all(d.path().join("nested")).unwrap();
    symlink(outside.path(), d.path().join("nested")).unwrap();
    assert_eq!(
        repo.snapshot(&AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Unsupported
    );
    assert_eq!(
        fs::read(outside.path().join("file")).unwrap(),
        b"secret outside\n"
    );
}
#[test]
fn init_and_cancelled_preview_binding_and_timeout() {
    let d = tempfile::tempdir().unwrap();
    let mut s = GitService::new();
    send(
        &mut s,
        GitCommand::Init {
            path: d.path().into(),
            initial_branch: "main".into(),
        },
    );
    fs::write(d.path().join("a"), "a\n").unwrap();
    send(&mut s, GitCommand::Refresh);
    send(
        &mut s,
        GitCommand::Prepare(LocalAction::Stage {
            paths: vec!["a".into()],
        }),
    );
    let p = s.preview.clone().unwrap();
    s.dispatch(GitCommand::Cancel).unwrap();
    assert!(
        s.dispatch(GitCommand::Apply {
            preview_id: p.id,
            repo_generation: p.generation
        })
        .is_err()
    );
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let err = repo
        .run_timeout(
            &["status".into()],
            None,
            &AtomicBool::new(false),
            Duration::ZERO,
        )
        .unwrap_err();
    assert_eq!(err.kind, GitErrorKind::Timeout);
}
#[test]
fn automatic_status_does_not_run_submodule_filters_but_keeps_staged_gitlink() {
    let child = fixture();
    base(child.path());
    let parent = fixture();
    base(parent.path());
    git(
        parent.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            child.path().to_str().unwrap(),
            "module",
        ],
    );
    git(parent.path(), &["commit", "-m", "submodule"]);
    let sub = parent.path().join("module");
    git(&sub, &["config", "user.name", "Fixture"]);
    git(&sub, &["config", "user.email", "fixture@example.invalid"]);
    git(&sub, &["config", "commit.gpgsign", "false"]);
    fs::write(sub.join("a.txt"), "second\n").unwrap();
    git(&sub, &["commit", "-am", "second"]);
    git(parent.path(), &["add", "module"]);
    let marker = parent.path().join("submodule-filter-ran");
    let hook = parent.path().join("empty-hooks/helper");
    fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(
        &sub,
        &["config", "filter.danger.clean", hook.to_str().unwrap()],
    );
    fs::write(sub.join(".gitattributes"), "a.txt filter=danger\n").unwrap();
    fs::write(sub.join("a.txt"), "dirty in submodule\n").unwrap();
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: parent.path().into(),
            trusted: false,
        },
    );
    assert!(!marker.exists());
    let snapshot = service.snapshot.as_ref().unwrap();
    let entry = snapshot
        .entries
        .iter()
        .find(|e| e.path == Path::new("module"))
        .unwrap();
    assert_eq!(entry.index, 'M');
    assert!(entry.submodule.starts_with('S'));
    assert!(!marker.exists());
}
#[test]
fn reference_history_paginates_beyond_200_and_pins_topology_when_branch_moves() {
    let d = fixture();
    base(d.path());
    let first = git(d.path(), &["rev-parse", "HEAD"]);
    let tree = git(d.path(), &["rev-parse", "HEAD^{tree}"]);
    let mut ids = vec![first.clone()];
    let mut parent = first.clone();
    // Real commit objects without changing the index/worktree or invoking hooks.
    for index in 1..=215 {
        parent = git(
            d.path(),
            &[
                "commit-tree",
                &tree,
                "-p",
                &parent,
                "-m",
                &format!("selected-{index:03}"),
            ],
        );
        ids.push(parent.clone());
    }
    git(d.path(), &["update-ref", "refs/heads/selected", &parent]);
    let unrelated = git(d.path(), &["commit-tree", &tree, "-m", "unrelated-root"]);
    git(
        d.path(),
        &["update-ref", "refs/heads/unrelated", &unrelated],
    );
    let expected = ids.iter().rev().cloned().collect::<Vec<_>>();
    let mut service = open(d.path());
    send(
        &mut service,
        GitCommand::HistoryReference {
            skip: 0,
            limit: 200,
            path: None,
            reference: Some("refs/heads/selected".into()),
        },
    );
    let first_page = service.history.clone().unwrap();
    assert_eq!(
        first_page.resolved_reference.as_deref(),
        Some(parent.as_str())
    );
    assert!(first_page.has_more);
    assert_eq!(
        first_page
            .commits
            .iter()
            .map(|c| c.oid.clone())
            .collect::<Vec<_>>(),
        expected[..200]
    );
    assert!(!first_page.commits.iter().any(|c| c.oid == unrelated));
    // Move the branch after the first page. The next request is anchored to the
    // resolved OID returned by that page, retaining the original ancestor chain.
    git(d.path(), &["update-ref", "refs/heads/selected", &first]);
    send(
        &mut service,
        GitCommand::HistoryReference {
            skip: 200,
            limit: 200,
            path: None,
            reference: first_page.resolved_reference.clone(),
        },
    );
    let next = service.history.as_ref().unwrap();
    assert!(!next.has_more);
    assert_eq!(
        next.commits
            .iter()
            .map(|c| c.oid.clone())
            .collect::<Vec<_>>(),
        expected[200..]
    );
    assert!(next.commits.iter().all(|c| c.lane == 0));
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let moved = repo
        .history_reference(
            0,
            200,
            None,
            Some("refs/heads/selected"),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(moved.commits.len(), 1);
    let file = repo
        .history_reference(
            0,
            200,
            Some(Path::new("a.txt")),
            Some(&parent),
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(file.commits.len(), 1);
    assert_eq!(file.commits[0].oid, first);
    assert!(
        repo.history_reference(0, 200, None, Some("--all"), &AtomicBool::new(false))
            .is_err()
    );
    assert!(
        repo.history_reference(
            0,
            200,
            None,
            Some("refs/heads/missing"),
            &AtomicBool::new(false)
        )
        .is_err()
    );
}

#[test]
fn history_replacement_retains_last_page_until_ack_and_clears_on_failure() {
    let d = fixture();
    base(d.path());
    let mut service = open(d.path());
    send(
        &mut service,
        GitCommand::HistoryReference {
            skip: 0,
            limit: 200,
            path: None,
            reference: Some("refs/heads/main".into()),
        },
    );
    let first = service.history.as_ref().unwrap().commits[0].oid.clone();
    service
        .dispatch(GitCommand::HistoryReference {
            skip: 0,
            limit: 200,
            path: None,
            reference: Some("HEAD".into()),
        })
        .unwrap();
    assert!(service.history_pending());
    assert_eq!(service.history.as_ref().unwrap().commits[0].oid, first);
    settle(&mut service);
    assert!(!service.history_pending());
    assert_eq!(service.history.as_ref().unwrap().commits[0].oid, first);
    service
        .dispatch(GitCommand::HistoryReference {
            skip: 0,
            limit: 200,
            path: None,
            reference: Some("refs/heads/not-existing".into()),
        })
        .unwrap();
    assert!(service.history_pending());
    assert!(service.history.is_some());
    settle(&mut service);
    assert!(!service.history_pending());
    assert!(service.error.is_some());
    assert!(service.history.is_none());
    assert_eq!(git(d.path(), &["branch", "--show-current"]), "main");
    assert!(git(d.path(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn staging_preserves_racy_index_timestamp_for_same_size_edits() {
    let d = fixture();
    let root = d.path();
    git(root, &["config", "core.trustctime", "false"]);
    git(root, &["config", "core.checkStat", "minimal"]);
    let file = root.join("a.txt");
    let timestamp = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
    let set_time = |path: &Path| {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(timestamp))
            .unwrap();
    };
    fs::write(&file, "base\n").unwrap();
    set_time(&file);
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-m", "Timestamp fixture"]);
    fs::write(&file, "next\n").unwrap();
    set_time(&file);
    // Controlled coarse-clock case: Git must compare contents, not only stat.
    set_time(&root.join(".git/index"));
    let mut service = open(root);
    assert!(
        service
            .snapshot
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.path == Path::new("a.txt"))
    );
    apply(
        &mut service,
        LocalAction::Stage {
            paths: vec!["a.txt".into()],
        },
    );
    assert_eq!(git(root, &["show", ":a.txt"]), "next");
    assert_eq!(git(root, &["show", "HEAD:a.txt"]), "base");
    assert_eq!(fs::read_to_string(file).unwrap(), "next\n");
}
