use codisseu_git::{discovery, preferences, scm, shelf, workflows, *};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8_lossy(&result.stdout).trim().into()
}

#[test]
fn declared_roots_are_discoverable_without_an_active_repository_and_open_scope_resets_trust() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    init(&first);
    init(&second);
    let workspace = dir.path().join("team.code-workspace");
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::DiscoverRoots {
            workspace: workspace.clone(),
            roots: vec![first.clone(), second.clone()],
        },
    );
    assert!(service.repo.is_none());
    assert_eq!(service.repositories.len(), 2);
    fs::write(first.join("a.txt"), "changed\n").unwrap();
    send(
        &mut service,
        GitCommand::Open {
            path: first.clone(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::Prepare(LocalAction::Stage {
            paths: vec!["a.txt".into()],
        }),
    );
    let old = service.preview.clone().unwrap();
    send(
        &mut service,
        GitCommand::OpenRoots {
            workspace,
            roots: vec![second.clone()],
        },
    );
    assert!(service.repo.is_none());
    assert!(service.snapshot.is_none());
    assert!(service.preview.is_none());
    assert!(!service.trusted);
    assert_eq!(service.repositories.len(), 1);
    assert_eq!(service.repositories[0].root, second);
    assert!(
        service
            .dispatch(GitCommand::Apply {
                preview_id: old.id,
                repo_generation: old.generation
            })
            .is_err()
    );
    let empty = dir.path().join("empty.code-workspace");
    send(
        &mut service,
        GitCommand::OpenRoots {
            workspace: empty,
            roots: vec![],
        },
    );
    assert!(service.repo.is_none());
    assert!(service.repositories.is_empty());
    assert!(service.error.is_none());
    assert_eq!(git(&first, &["diff", "--cached", "--name-only"]), "");
    assert_eq!(
        fs::read_to_string(first.join("a.txt")).unwrap(),
        "changed\n"
    );
    assert_eq!(git(&second, &["status", "--porcelain"]), "");
}

#[test]
fn explicit_project_roots_have_priority_over_incidental_nested_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("documents");
    fs::create_dir(&docs).unwrap();
    let nested = docs.join("nested");
    init(&nested);
    let project = dir.path().join("project");
    init(&project);
    let excluded = dir.path().join("excluded");
    init(&excluded);
    let found = discovery::discover_roots(
        &dir.path().join("team.code-workspace"),
        &[docs, project.clone()],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        found
            .repositories
            .iter()
            .map(|r| &r.root)
            .collect::<Vec<_>>(),
        vec![&project, &nested]
    );
    assert!(!found.repositories.iter().any(|r| r.root == excluded));
    assert!(found.repositories.iter().all(|r| r.entries.is_some()));
    assert!(found.errors.is_empty());
}
fn init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-b", "main"]);
    let hooks = root.join(".git/fixture-hooks");
    fs::create_dir(&hooks).unwrap();
    for (key, value) in [
        ("user.name", "SCM Fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", hooks.to_str().unwrap()),
    ] {
        git(root, &["config", key, value]);
    }
    fs::write(root.join("a.txt"), "base\n").unwrap();
    fs::write(root.join("b.txt"), "base\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "Initial"]);
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    dir
}
fn settle(service: &mut GitService) {
    let start = Instant::now();
    while service.is_busy() {
        service.poll();
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "{:?}",
            service.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
fn send(service: &mut GitService, command: GitCommand) {
    service.dispatch(command).unwrap();
    settle(service);
    assert!(service.error.is_none(), "{:?}", service.error);
}
fn apply(service: &mut GitService) {
    let preview = service.preview.clone().unwrap();
    send(
        service,
        GitCommand::Apply {
            preview_id: preview.id,
            repo_generation: preview.generation,
        },
    );
    assert!(
        service.report.as_ref().unwrap().success,
        "{:?}",
        service.report
    );
}
#[test]
fn remote_actions_are_real_and_config_changes_invalidate_preview() {
    let dir = fixture();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let add = scm::prepare_scm(
        &repo,
        ScmAction::RemoteAdd {
            name: "origin".into(),
            url: "/tmp/codisseu-unused-own-remote".into(),
        },
        &cancel,
    )
    .unwrap();
    assert!(scm::apply_scm(&repo, &add, &cancel).unwrap().success);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "origin"]),
        "/tmp/codisseu-unused-own-remote"
    );
    let rename = scm::prepare_scm(
        &repo,
        ScmAction::RemoteRename {
            name: "origin".into(),
            new_name: "backup".into(),
        },
        &cancel,
    )
    .unwrap();
    git(
        dir.path(),
        &["config", "remote.origin.url", "/tmp/changed-externally"],
    );
    assert_eq!(
        scm::apply_scm(&repo, &rename, &cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    assert_eq!(git(dir.path(), &["remote"]), "origin");
    let rename = scm::prepare_scm(
        &repo,
        ScmAction::RemoteRename {
            name: "origin".into(),
            new_name: "backup".into(),
        },
        &cancel,
    )
    .unwrap();
    assert!(scm::apply_scm(&repo, &rename, &cancel).unwrap().success);
    let edit = scm::prepare_scm(
        &repo,
        ScmAction::RemoteSetUrl {
            name: "backup".into(),
            url: "/tmp/push-only".into(),
            push: true,
        },
        &cancel,
    )
    .unwrap();
    assert!(scm::apply_scm(&repo, &edit, &cancel).unwrap().success);
    assert_eq!(
        git(dir.path(), &["remote", "get-url", "--push", "backup"]),
        "/tmp/push-only"
    );
    let remove = scm::prepare_scm(
        &repo,
        ScmAction::RemoteRemove {
            name: "backup".into(),
        },
        &cancel,
    )
    .unwrap();
    assert!(remove.preview.destructive);
    assert!(scm::apply_scm(&repo, &remove, &cancel).unwrap().success);
    assert_eq!(git(dir.path(), &["remote"]), "");
    assert!(
        scm::prepare_scm(
            &repo,
            ScmAction::RemoteAdd {
                name: "bad".into(),
                url: "https://user:secret@example.invalid/repo".into()
            },
            &cancel
        )
        .is_err()
    );
}
#[test]
fn selected_commit_signoff_and_coauthors_preserve_other_index_entries() {
    let dir = fixture();
    fs::write(dir.path().join("a.txt"), "selected\n").unwrap();
    fs::write(dir.path().join("b.txt"), "excluded staged\n").unwrap();
    git(dir.path(), &["add", "b.txt"]);
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: dir.path().into(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::ScmPrepare(ScmAction::CommitOptions {
            selection: CommitSelection::WorkingPaths(vec!["a.txt".into()]),
            message: "Commit from SCM\n\nPreserve body # literal".into(),
            amend: false,
            signoff: true,
            coauthors: vec![CommitIdentity {
                name: "Co Author".into(),
                email: "co@example.invalid".into(),
            }],
        }),
    );
    apply(&mut service);
    let message = git(dir.path(), &["log", "-1", "--format=%B"]);
    assert!(message.contains("Signed-off-by: SCM Fixture <fixture@example.invalid>"));
    assert!(message.contains("Co-authored-by: Co Author <co@example.invalid>"));
    assert!(message.contains("Preserve body # literal"));
    assert_eq!(
        git(
            dir.path(),
            &[
                "log",
                "-1",
                "--format=%(trailers:key=Co-authored-by,valueonly)"
            ]
        ),
        "Co Author <co@example.invalid>"
    );
    assert_eq!(git(dir.path(), &["show", "HEAD:a.txt"]), "selected");
    assert_eq!(git(dir.path(), &["show", "HEAD:b.txt"]), "base");
    assert_eq!(
        git(dir.path(), &["diff", "--cached", "--name-only"]),
        "b.txt"
    );
}
#[test]
fn upstream_configuration_uses_the_selected_local_and_full_remote_ref() {
    let dir = fixture();
    git(
        dir.path(),
        &["remote", "add", "origin", "/tmp/unused-own-bare"],
    );
    git(
        dir.path(),
        &["update-ref", "refs/remotes/origin/release", "HEAD"],
    );
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let plan = scm::prepare_scm(
        &repo,
        ScmAction::SetUpstream {
            branch: "refs/heads/main".into(),
            upstream: Some("refs/remotes/origin/release".into()),
        },
        &cancel,
    )
    .unwrap();
    assert!(scm::apply_scm(&repo, &plan, &cancel).unwrap().success);
    assert_eq!(
        git(
            dir.path(),
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        "refs/remotes/origin/release"
    );
    let clear = scm::prepare_scm(
        &repo,
        ScmAction::SetUpstream {
            branch: "main".into(),
            upstream: None,
        },
        &cancel,
    )
    .unwrap();
    assert!(scm::apply_scm(&repo, &clear, &cancel).unwrap().success);
    assert_eq!(
        git(
            dir.path(),
            &["for-each-ref", "--format=%(upstream)", "refs/heads/main"]
        )
        .lines()
        .count(),
        0
    );
}
#[test]
fn favorite_refs_persist_per_worktree_and_real_checkout_records_recent() {
    let dir = fixture();
    git(dir.path(), &["branch", "topic"]);
    git(dir.path(), &["branch", "origin/topic"]);
    git(
        dir.path(),
        &["update-ref", "refs/remotes/origin/topic", "HEAD"],
    );
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    preferences::set_favorite(&repo, "refs/heads/origin/topic", true, &cancel).unwrap();
    preferences::set_favorite(&repo, "refs/remotes/origin/topic", true, &cancel).unwrap();
    assert_eq!(
        preferences::load(&repo, &cancel).unwrap().favorites.len(),
        2
    );
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::Open {
            path: dir.path().into(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::Prepare(LocalAction::CheckoutBranch {
            name: "topic".into(),
        }),
    );
    apply(&mut service);
    send(&mut service, GitCommand::BranchPreferences);
    assert_eq!(service.branch_preferences.recent[0], "refs/heads/topic");
    assert_eq!(service.branch_preferences.favorites.len(), 2);
    let worktree = dir
        .path()
        .parent()
        .unwrap()
        .join(format!("codisseu-other-worktree-{}", std::process::id()));
    git(
        dir.path(),
        &["worktree", "add", "-b", "other", worktree.to_str().unwrap()],
    );
    let linked = Repository::discover(&worktree, &cancel).unwrap();
    assert!(
        preferences::load(&linked, &cancel)
            .unwrap()
            .favorites
            .is_empty()
    );
    git(
        dir.path(),
        &["worktree", "remove", worktree.to_str().unwrap()],
    );
}
#[test]
fn discovery_is_bounded_and_does_not_follow_links_and_survives_selection() {
    let dir = tempfile::tempdir().unwrap();
    init(&dir.path().join("a"));
    init(&dir.path().join("ação"));
    let outside = fixture();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();
    let cancel = AtomicBool::new(false);
    let discovery = discovery::discover_repositories(dir.path(), 3, 16, 1000, &cancel).unwrap();
    assert_eq!(discovery.repositories.len(), 2);
    assert!(discovery.repositories.iter().all(|repo| {
        repo.changes == Some(0)
            && repo
                .entries
                .as_ref()
                .is_some_and(|entries| entries.is_empty())
            && !repo.entries_truncated
    }));
    let limited = discovery::discover_repositories(dir.path(), 3, 1, 1000, &cancel).unwrap();
    assert_eq!(limited.repositories.len(), 1);
    assert!(limited.truncated);
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::DiscoverRepositories {
            workspace: dir.path().into(),
            max_depth: 3,
            max_repositories: 16,
            max_entries: 1000,
        },
    );
    send(
        &mut service,
        GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: dir.path().join("a"),
            trusted: false,
        },
    );
    assert_eq!(service.repositories.len(), 2);
    assert_eq!(
        service
            .repositories
            .iter()
            .find(|repo| repo.root.ends_with("a"))
            .unwrap()
            .changes,
        Some(0)
    );
    send(
        &mut service,
        GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: dir.path().join("ação"),
            trusted: false,
        },
    );
    assert_eq!(service.repositories.len(), 2);
    service
        .dispatch(GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: outside.path().into(),
            trusted: false,
        })
        .unwrap();
    settle(&mut service);
    assert!(
        service
            .error
            .as_ref()
            .is_some_and(|message| message.contains("dentro"))
    );
    assert!(service.repo.is_none());
    assert_eq!(
        discovery::discover_repositories(dir.path(), 3, 16, 1000, &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        GitErrorKind::Cancelled
    );
}
#[test]
fn stash_creation_timestamp_matches_the_actual_git_commit() {
    let dir = fixture();
    fs::write(dir.path().join("a.txt"), "stash me\n").unwrap();
    git(dir.path(), &["stash", "push", "-m", "Timestamp fixture"]);
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let stash = workflows::list_stashes(&repo, &cancel).unwrap();
    assert_eq!(stash.len(), 1);
    assert_eq!(
        stash[0].created_unix,
        git(dir.path(), &["show", "-s", "--format=%ct", "refs/stash"])
            .parse::<i64>()
            .unwrap()
    );
}
#[test]
fn shelf_reports_rename_delete_and_layers_and_recovers_deleted_shelf() {
    let dir = fixture();
    git(dir.path(), &["mv", "a.txt", "renamed.txt"]);
    fs::remove_file(dir.path().join("b.txt")).unwrap();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let prepared = shelf::prepare_shelf(
        &repo,
        ShelfAction::Shelve {
            name: "Rename and delete".into(),
            paths: vec!["renamed.txt".into(), "b.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    let report = shelf::apply_shelf(&repo, &prepared, &cancel).unwrap();
    assert!(report.success);
    let entries = shelf::list_shelves(&repo, &cancel).unwrap();
    let info = &entries[0];
    assert!(
        info.files
            .iter()
            .any(|file| file.path == Path::new("renamed.txt")
                && file.original_path.as_deref() == Some(Path::new("a.txt"))
                && file.index_status == 'R')
    );
    assert!(
        info.files
            .iter()
            .any(|file| file.path == Path::new("b.txt") && file.status == 'D')
    );
    let id = info.id.clone();
    let before = git(dir.path(), &["status", "--porcelain"]);
    let delete =
        shelf::prepare_shelf(&repo, ShelfAction::Delete { id: id.clone() }, &cancel).unwrap();
    assert!(shelf::apply_shelf(&repo, &delete, &cancel).unwrap().success);
    assert!(shelf::list_shelves(&repo, &cancel).unwrap().is_empty());
    let deleted = shelf::list_deleted_shelves(&repo, &cancel).unwrap();
    assert_eq!(deleted[0].id, id);
    assert!(deleted[0].deleted_unix.is_some());
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), before);
    let restore = shelf::prepare_shelf(
        &repo,
        ShelfAction::RestoreDeleted { id: id.clone() },
        &cancel,
    )
    .unwrap();
    assert!(
        shelf::apply_shelf(&repo, &restore, &cancel)
            .unwrap()
            .success
    );
    assert!(
        shelf::list_deleted_shelves(&repo, &cancel)
            .unwrap()
            .is_empty()
    );
    let apply = shelf::prepare_shelf(&repo, ShelfAction::Unshelve { id }, &cancel).unwrap();
    assert!(shelf::apply_shelf(&repo, &apply, &cancel).unwrap().success);
    assert_eq!(
        fs::read_to_string(dir.path().join("renamed.txt")).unwrap(),
        "base\n"
    );
    assert!(!dir.path().join("a.txt").exists());
    assert!(!dir.path().join("b.txt").exists());
}

#[test]
fn unshelve_subset_preserves_layers_and_creates_real_changelist_and_remaining_patch() {
    let dir = fixture();
    for name in ["a.txt", "b.txt"] {
        fs::write(dir.path().join(name), format!("index {name}\n")).unwrap();
        git(dir.path(), &["add", name]);
        fs::write(dir.path().join(name), format!("working {name}\n")).unwrap();
    }
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let save = shelf::prepare_shelf(
        &repo,
        ShelfAction::Shelve {
            name: "Two layers, two files".into(),
            paths: vec!["a.txt".into(), "b.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    assert!(shelf::apply_shelf(&repo, &save, &cancel).unwrap().success);
    let id = shelf::list_shelves(&repo, &cancel).unwrap()[0].id.clone();
    fs::write(dir.path().join("unrelated.txt"), "keep in Changes\n").unwrap();
    let plan = shelf::prepare_shelf(
        &repo,
        ShelfAction::UnshelveOptions {
            id: id.clone(),
            paths: Some(vec!["a.txt".into()]),
            changelist_name: Some("Review target".into()),
            comment: Some("Restored selection\nTwo layers".into()),
            set_active: true,
            remove_applied: true,
        },
        &cancel,
    )
    .unwrap();
    let report = shelf::apply_shelf(&repo, &plan, &cancel).unwrap();
    assert!(report.success, "{report:?}");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "working a.txt\n"
    );
    assert_eq!(git(dir.path(), &["show", ":a.txt"]), "index a.txt");
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "base\n"
    );
    let lists = shelf::list_changelists(&repo, &cancel).unwrap();
    let target = lists
        .iter()
        .find(|list| list.name == "Review target")
        .unwrap();
    assert!(target.is_default);
    assert_eq!(target.comment, "Restored selection\nTwo layers");
    assert_eq!(target.paths, vec![std::path::PathBuf::from("a.txt")]);
    assert!(
        lists
            .iter()
            .find(|list| list.name == "Changes")
            .unwrap()
            .paths
            .contains(&"unrelated.txt".into())
    );
    let details = shelf::shelf_details(&repo, &id, &cancel).unwrap();
    assert_eq!(details.info.paths, vec![std::path::PathBuf::from("b.txt")]);
    assert!(!String::from_utf8_lossy(&details.patch).contains("diff --git a/a.txt"));
    assert!(String::from_utf8_lossy(&details.staged_patch).contains("index b.txt"));
    assert!(String::from_utf8_lossy(&details.working_patch).contains("working b.txt"));
    let rest = shelf::prepare_shelf(
        &repo,
        ShelfAction::UnshelveOptions {
            id: id.clone(),
            paths: None,
            changelist_name: None,
            comment: None,
            set_active: false,
            remove_applied: true,
        },
        &cancel,
    )
    .unwrap();
    assert!(shelf::apply_shelf(&repo, &rest, &cancel).unwrap().success);
    assert!(shelf::list_shelves(&repo, &cancel).unwrap().is_empty());
    assert_eq!(
        shelf::list_deleted_shelves(&repo, &cancel).unwrap()[0].id,
        id
    );
    assert_eq!(git(dir.path(), &["show", ":b.txt"]), "index b.txt");
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "working b.txt\n"
    );
    assert_eq!(git(dir.path(), &["show", ":a.txt"]), "index a.txt");
}
#[test]
fn remove_applied_retains_conflicted_files_and_their_original_layers() {
    let dir = fixture();
    fs::write(dir.path().join("a.txt"), "shelf a\n").unwrap();
    fs::write(dir.path().join("b.txt"), "shelf b\n").unwrap();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let save = shelf::prepare_shelf(
        &repo,
        ShelfAction::Shelve {
            name: "Conflict one, restore one".into(),
            paths: vec!["a.txt".into(), "b.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    assert!(shelf::apply_shelf(&repo, &save, &cancel).unwrap().success);
    let id = shelf::list_shelves(&repo, &cancel).unwrap()[0].id.clone();
    fs::write(dir.path().join("a.txt"), "local a\n").unwrap();
    let plan = shelf::prepare_shelf(
        &repo,
        ShelfAction::UnshelveOptions {
            id: id.clone(),
            paths: None,
            changelist_name: None,
            comment: None,
            set_active: false,
            remove_applied: true,
        },
        &cancel,
    )
    .unwrap();
    let report = shelf::apply_shelf(&repo, &plan, &cancel).unwrap();
    assert!(report.success);
    assert_eq!(report.conflicts, vec![std::path::PathBuf::from("a.txt")]);
    assert!(report.recovery_path.unwrap().exists());
    let remaining = shelf::shelf_details(&repo, &id, &cancel).unwrap();
    assert_eq!(
        remaining.info.paths,
        vec![std::path::PathBuf::from("a.txt")]
    );
    assert!(String::from_utf8_lossy(&remaining.working_patch).contains("shelf a"));
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "shelf b\n"
    );
    assert_eq!(
        git(dir.path(), &["ls-files", "--unmerged"]).lines().count(),
        3
    );
}
#[test]
fn deleted_shelf_details_and_explicit_permanent_delete_preserve_worktree() {
    let dir = fixture();
    fs::write(dir.path().join("a.txt"), "saved\n").unwrap();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let save = shelf::prepare_shelf(
        &repo,
        ShelfAction::Shelve {
            name: "Delete lifecycle".into(),
            paths: vec!["a.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    shelf::apply_shelf(&repo, &save, &cancel).unwrap();
    let id = shelf::list_shelves(&repo, &cancel).unwrap()[0].id.clone();
    let remove =
        shelf::prepare_shelf(&repo, ShelfAction::Delete { id: id.clone() }, &cancel).unwrap();
    shelf::apply_shelf(&repo, &remove, &cancel).unwrap();
    let details = shelf::shelf_deleted_details(&repo, &id, &cancel).unwrap();
    assert!(details.info.deleted_unix.is_some());
    assert!(String::from_utf8_lossy(&details.patch).contains("saved"));
    let permanent =
        shelf::prepare_shelf(&repo, ShelfAction::DeletePermanently { id }, &cancel).unwrap();
    assert!(permanent.preview.destructive);
    assert!(
        shelf::apply_shelf(&repo, &permanent, &cancel)
            .unwrap()
            .success
    );
    assert!(
        shelf::list_deleted_shelves(&repo, &cancel)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "base\n"
    );
}

#[test]
fn explicit_delete_untracked_is_bounded_backed_up_and_never_git_clean() {
    let dir = fixture();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    fs::write(dir.path().join("--new ação.bin"), b"owned\0binary\xff\n").unwrap();
    fs::write(dir.path().join("keep.txt"), "not selected\n").unwrap();
    let before_index = git(dir.path(), &["write-tree"]);
    let plan = scm::prepare_scm(
        &repo,
        ScmAction::DeleteUntracked {
            paths: vec!["--new ação.bin".into()],
        },
        &cancel,
    )
    .unwrap();
    assert!(plan.preview.destructive);
    let report = scm::apply_scm(&repo, &plan, &cancel).unwrap();
    assert!(report.success, "{report:?}");
    assert!(!dir.path().join("--new ação.bin").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
        "not selected\n"
    );
    assert_eq!(git(dir.path(), &["write-tree"]), before_index);
    let recovery = dir.path().join(".git/codisseu/untracked-recovery");
    let batch = fs::read_dir(recovery)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read(batch.join("0.bin")).unwrap(),
        b"owned\0binary\xff\n"
    );
    assert_eq!(
        fs::read(batch.join("0.removed")).unwrap(),
        b"owned\0binary\xff\n"
    );
    assert!(
        fs::read_to_string(batch.join("metadata.json"))
            .unwrap()
            .contains("sha256")
    );
    assert!(
        scm::prepare_scm(
            &repo,
            ScmAction::DeleteUntracked {
                paths: vec!["a.txt".into()]
            },
            &cancel
        )
        .is_err()
    );
    fs::create_dir(dir.path().join("folder")).unwrap();
    fs::write(dir.path().join("folder/inside"), "keep\n").unwrap();
    assert!(
        scm::prepare_scm(
            &repo,
            ScmAction::DeleteUntracked {
                paths: vec!["folder".into()]
            },
            &cancel
        )
        .is_err()
    );
    assert!(dir.path().join("folder/inside").exists());
}
#[test]
fn deleting_untracked_rejects_external_edits_staging_links_and_changed_preview() {
    let dir = fixture();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    fs::write(dir.path().join("new.txt"), "before\n").unwrap();
    let plan = scm::prepare_scm(
        &repo,
        ScmAction::DeleteUntracked {
            paths: vec!["new.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    fs::write(dir.path().join("new.txt"), "external edit\n").unwrap();
    assert_eq!(
        scm::apply_scm(&repo, &plan, &cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("new.txt")).unwrap(),
        "external edit\n"
    );
    let mut plan = scm::prepare_scm(
        &repo,
        ScmAction::DeleteUntracked {
            paths: vec!["new.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    plan.preview.summary = "Innocent read".into();
    assert_eq!(
        scm::apply_scm(&repo, &plan, &cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    let plan = scm::prepare_scm(
        &repo,
        ScmAction::DeleteUntracked {
            paths: vec!["new.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    git(dir.path(), &["add", "new.txt"]);
    assert_eq!(
        scm::apply_scm(&repo, &plan, &cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    assert!(dir.path().join("new.txt").exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("new.txt", dir.path().join("link")).unwrap();
        assert!(
            scm::prepare_scm(
                &repo,
                ScmAction::DeleteUntracked {
                    paths: vec!["link".into()]
                },
                &cancel
            )
            .is_err()
        );
    }
    fs::write(dir.path().join("another"), "keep\n").unwrap();
    let plan = scm::prepare_scm(
        &repo,
        ScmAction::DeleteUntracked {
            paths: vec!["another".into()],
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(
        scm::apply_scm(&repo, &plan, &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        GitErrorKind::Cancelled
    );
    assert!(dir.path().join("another").exists());
}
#[test]
fn favorite_removal_handles_a_deleted_ref_and_metadata_links_are_refused() {
    let dir = fixture();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    git(dir.path(), &["branch", "obsolete"]);
    preferences::set_favorite(&repo, "refs/heads/obsolete", true, &cancel).unwrap();
    git(dir.path(), &["branch", "-d", "obsolete"]);
    assert!(
        preferences::set_favorite(&repo, "refs/heads/obsolete", false, &cancel)
            .unwrap()
            .favorites
            .is_empty()
    );
    #[cfg(unix)]
    {
        let metadata = dir.path().join(".git/codisseu/branches.json");
        fs::remove_file(&metadata).unwrap();
        let outside = dir.path().join("outside-preferences");
        fs::write(&outside, "do not touch").unwrap();
        std::os::unix::fs::symlink(&outside, &metadata).unwrap();
        assert!(preferences::load(&repo, &cancel).is_err());
        assert!(preferences::set_favorite(&repo, "refs/heads/main", true, &cancel).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "do not touch");
    }
}

#[test]
fn unshelve_remove_all_refuses_full_recently_deleted_before_restoring() {
    fn copy_bundle(source: &Path, destination: &Path) {
        fs::create_dir(destination).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_bundle(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let dir = fixture();
    fs::write(dir.path().join("a.txt"), "quota protected\n").unwrap();
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    let save = shelf::prepare_shelf(
        &repo,
        ShelfAction::Shelve {
            name: "Quota fixture".into(),
            paths: vec!["a.txt".into()],
        },
        &cancel,
    )
    .unwrap();
    assert!(shelf::apply_shelf(&repo, &save, &cancel).unwrap().success);
    let id = shelf::list_shelves(&repo, &cancel).unwrap()[0].id.clone();
    let source = dir.path().join(".git/codisseu/shelves").join(&id);
    let deleted = dir.path().join(".git/codisseu/deleted");
    fs::create_dir(&deleted).unwrap();
    for number in 0..128 {
        let archived_id = format!("{number:032x}");
        let destination = deleted.join(&archived_id);
        copy_bundle(&source, &destination);
        let path = destination.join("metadata.json");
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        metadata["id"] = archived_id.into();
        fs::write(path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        fs::write(destination.join("deleted-at.json"), "1").unwrap();
    }
    assert_eq!(
        shelf::list_deleted_shelves(&repo, &cancel).unwrap().len(),
        128
    );
    let before = repo.snapshot(&cancel).unwrap();
    let error = shelf::prepare_shelf(
        &repo,
        ShelfAction::UnshelveOptions {
            id: id.clone(),
            paths: None,
            changelist_name: None,
            comment: None,
            set_active: false,
            remove_applied: true,
        },
        &cancel,
    )
    .unwrap_err();
    assert!(error.message.contains("128"), "{error:?}");
    assert_eq!(repo.snapshot(&cancel).unwrap(), before);
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(shelf::list_shelves(&repo, &cancel).unwrap()[0].id, id);
}

#[test]
fn ten_repository_status_caches_are_real_independent_and_never_authorize_stale_writes() {
    let dir = tempfile::tempdir().unwrap();
    for number in 0..10 {
        let root = dir.path().join(format!("project-{number:02}"));
        init(&root);
        fs::write(root.join("a.txt"), format!("working {number}\n")).unwrap();
        fs::write(root.join("b.txt"), format!("staged {number}\n")).unwrap();
        git(&root, &["add", "b.txt"]);
        fs::write(root.join(format!("ação-{number}.txt")), "untracked\n").unwrap();
    }
    // A lightweight listing must not hash/read a70MiB untracked payload.
    let huge = dir.path().join("project-09/large-untracked.bin");
    fs::File::create(&huge)
        .unwrap()
        .set_len(70 * 1024 * 1024)
        .unwrap();
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::DiscoverRepositories {
            workspace: dir.path().into(),
            max_depth: 6,
            max_repositories: 32,
            max_entries: 50000,
        },
    );
    assert_eq!(service.repositories.len(), 10);
    for (number, summary) in service.repositories.iter().enumerate() {
        let entries = summary.entries.as_ref().unwrap();
        assert!(!summary.entries_truncated);
        assert_eq!(summary.changes, Some(if number == 9 { 4 } else { 3 }));
        assert!(entries.iter().any(|entry| entry.path == Path::new("a.txt")
            && entry.index == '.'
            && entry.worktree == 'M'));
        assert!(entries.iter().any(|entry| entry.path == Path::new("b.txt")
            && entry.index == 'M'
            && entry.worktree == '.'));
        assert!(entries.iter().any(
            |entry| entry.path == Path::new(&format!("ação-{number}.txt")) && entry.untracked
        ));
        assert_eq!(summary.ahead, None);
        assert_eq!(summary.behind, None);
    }
    let unchanged = service.repositories[1].entries.clone().unwrap();
    let root = dir.path().join("project-00");
    send(
        &mut service,
        GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: root.clone(),
            trusted: true,
        },
    );
    assert!(std::sync::Arc::ptr_eq(
        &unchanged,
        service.repositories[1].entries.as_ref().unwrap()
    ));
    send(
        &mut service,
        GitCommand::Prepare(LocalAction::Stage {
            paths: vec!["a.txt".into()],
        }),
    );
    fs::write(root.join("a.txt"), "external change after review\n").unwrap();
    let preview = service.preview.clone().unwrap();
    service
        .dispatch(GitCommand::Apply {
            preview_id: preview.id,
            repo_generation: preview.generation,
        })
        .unwrap();
    settle(&mut service);
    assert!(!service.report.as_ref().unwrap().success);
    assert_eq!(git(&root, &["diff", "--cached", "--name-only"]), "b.txt");
    let before_cancel = service.repositories[1].entries.clone().unwrap();
    service
        .dispatch(GitCommand::DiscoverRepositories {
            workspace: dir.path().into(),
            max_depth: 6,
            max_repositories: 32,
            max_entries: 50000,
        })
        .unwrap();
    service.dispatch(GitCommand::Cancel).unwrap();
    settle(&mut service);
    assert!(service.error.is_some());
    assert_eq!(service.repositories.len(), 10);
    assert!(std::sync::Arc::ptr_eq(
        &before_cancel,
        service.repositories[1].entries.as_ref().unwrap()
    ));
}

#[test]
fn selected_repository_refresh_keeps_aggregate_status_cache_within_ten_thousand() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["first", "second"] {
        let root = dir.path().join(name);
        init(&root);
        for number in 0..6000 {
            fs::write(root.join(format!("untracked-{number:04}")), "").unwrap();
        }
    }
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::DiscoverRepositories {
            workspace: dir.path().into(),
            max_depth: 2,
            max_repositories: 32,
            max_entries: 50000,
        },
    );
    assert_eq!(service.repositories.len(), 2);
    assert_eq!(
        service
            .repositories
            .iter()
            .map(|summary| summary.entries.as_ref().map_or(0, |entries| entries.len()))
            .sum::<usize>(),
        10000
    );
    assert!(
        service
            .repositories
            .iter()
            .any(|summary| summary.entries_truncated && summary.changes.is_none())
    );
    for name in ["first", "second", "first"] {
        send(
            &mut service,
            GitCommand::OpenWithin {
                workspace: dir.path().into(),
                path: dir.path().join(name),
                trusted: false,
            },
        );
        let selected = service
            .repositories
            .iter()
            .find(|summary| summary.root.ends_with(name))
            .unwrap();
        assert_eq!(selected.entries.as_ref().unwrap().len(), 6000);
        assert!(!selected.entries_truncated);
        assert_eq!(selected.changes, Some(6000));
        assert_eq!(
            service
                .repositories
                .iter()
                .map(|summary| summary.entries.as_ref().map_or(0, |entries| entries.len()))
                .sum::<usize>(),
            10000
        );
        assert!(
            service
                .repositories
                .iter()
                .any(|summary| summary.entries_truncated && summary.changes.is_none())
        );
    }
}

#[test]
fn summary_upstream_counts_require_actual_git_branch_ab_header() {
    let dir = fixture();
    git(dir.path(), &["branch", "upstream"]);
    git(
        dir.path(),
        &["branch", "--set-upstream-to=upstream", "main"],
    );
    let cancel = AtomicBool::new(false);
    let repo = Repository::discover(dir.path(), &cancel).unwrap();
    assert!(repo.snapshot(&cancel).unwrap().upstream_counts_known);
    let discovered = discovery::discover_repositories(dir.path(), 1, 32, 1000, &cancel).unwrap();
    assert_eq!(discovered.repositories[0].ahead, Some(0));
    assert_eq!(discovered.repositories[0].behind, Some(0));
    git(dir.path(), &["branch", "-D", "upstream"]);
    assert!(!repo.snapshot(&cancel).unwrap().upstream_counts_known);
    let discovered = discovery::discover_repositories(dir.path(), 1, 32, 1000, &cancel).unwrap();
    assert_eq!(discovered.repositories[0].ahead, None);
    assert_eq!(discovered.repositories[0].behind, None);
}

#[test]
fn multi_project_discovery_isolates_broken_git_and_status_and_finds_worktrees() {
    let dir = tempfile::tempdir().unwrap();
    let healthy = dir.path().join("healthy");
    init(&healthy);
    let broken = dir.path().join("broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(".git"), "gitdir: missing-own-fixture\n").unwrap();
    let nested = broken.join("independent");
    init(&nested);
    let bad_status = dir.path().join("bad-status");
    init(&bad_status);
    fs::write(bad_status.join(".git/index"), "invalid fixture index").unwrap();
    let linked = dir.path().join("linked");
    git(
        &healthy,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::DiscoverRepositories {
            workspace: dir.path().into(),
            max_depth: 3,
            max_repositories: 32,
            max_entries: 1000,
        },
    );
    assert_eq!(service.repositories.len(), 4);
    assert_eq!(service.discovery_errors.len(), 2);
    for root in [&healthy, &nested, &linked] {
        let summary = service
            .repositories
            .iter()
            .find(|r| &r.root == root)
            .unwrap();
        assert_eq!(summary.changes, Some(0));
        assert!(!summary.entries_truncated);
    }
    let unreadable = service
        .repositories
        .iter()
        .find(|r| r.root == bad_status)
        .unwrap();
    assert_eq!(unreadable.changes, None);
    assert!(unreadable.entries_truncated);
    assert!(service.discovery_errors.iter().any(|e| e.path == broken));
    assert!(
        service
            .discovery_errors
            .iter()
            .any(|e| e.path == bad_status)
    );
    send(
        &mut service,
        GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: linked.clone(),
            trusted: true,
        },
    );
    assert!(service.repo.as_ref().unwrap().linked_worktree);
    assert_eq!(service.repo.as_ref().unwrap().root, linked);
}

#[test]
fn multi_project_discovery_diagnostics_are_bounded_and_cancellation_is_not_swallowed() {
    let dir = tempfile::tempdir().unwrap();
    for number in 0..40 {
        let root = dir.path().join(format!("invalid-{number}"));
        fs::create_dir(&root).unwrap();
        fs::write(root.join(".git"), "gitdir: missing-fixture\n").unwrap();
    }
    let result =
        discovery::discover_repositories(dir.path(), 2, 32, 1000, &AtomicBool::new(false)).unwrap();
    assert!(result.repositories.is_empty());
    assert_eq!(result.errors.len(), 32);
    assert!(result.truncated);
    assert!(
        result
            .errors
            .iter()
            .all(|e| e.message.chars().count() <= 2048)
    );
    assert_eq!(
        discovery::discover_repositories(dir.path(), 2, 32, 1000, &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        GitErrorKind::Cancelled
    );
}

#[test]
fn multi_project_commits_preserve_mixed_encoding_bytes_and_sibling_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let projects: [(&str, &[u8]); 3] = [
        ("utf8", "ação 😀\n".as_bytes()),
        ("latin1", b"a\xe7\xe3o caf\xe9\n"),
        ("windows1252", b"a\xe7\xe3o \x93caf\xe9\x94 \x80\n"),
    ];
    let roots: Vec<_> = projects
        .iter()
        .map(|(name, _)| dir.path().join(name))
        .collect();
    for (root, (_, bytes)) in roots.iter().zip(&projects) {
        init(root);
        fs::write(root.join("a.txt"), bytes).unwrap();
        fs::write(root.join("b.txt"), b"leave working version\n").unwrap();
    }
    let mut service = GitService::new();
    send(
        &mut service,
        GitCommand::DiscoverRepositories {
            workspace: dir.path().into(),
            max_depth: 2,
            max_repositories: 32,
            max_entries: 1000,
        },
    );
    assert_eq!(service.repositories.len(), 3);
    assert!(service.repositories.iter().all(|r| r.changes == Some(2)));
    for (root, (_, bytes)) in roots.iter().zip(&projects) {
        let sibling_states: Vec<_> = roots
            .iter()
            .filter(|sibling| *sibling != root)
            .map(|sibling| {
                (
                    sibling,
                    git(sibling, &["rev-parse", "HEAD"]),
                    git(sibling, &["write-tree"]),
                )
            })
            .collect();
        send(
            &mut service,
            GitCommand::OpenWithin {
                workspace: dir.path().into(),
                path: root.clone(),
                trusted: true,
            },
        );
        send(
            &mut service,
            GitCommand::Prepare(LocalAction::Stage {
                paths: vec!["a.txt".into()],
            }),
        );
        apply(&mut service);
        send(
            &mut service,
            GitCommand::Prepare(LocalAction::Commit {
                selection: CommitSelection::AllStaged,
                message: format!("Commit {}", root.file_name().unwrap().to_string_lossy()),
                amend: false,
            }),
        );
        apply(&mut service);
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["show", "HEAD:a.txt"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, *bytes);
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), *bytes);
        assert_eq!(git(root, &["show", "HEAD:b.txt"]), "base");
        assert_eq!(git(root, &["diff", "--name-only"]), "b.txt");
        for (sibling, head, index) in sibling_states {
            assert_eq!(git(sibling, &["rev-parse", "HEAD"]), head);
            assert_eq!(git(sibling, &["write-tree"]), index);
        }
    }
    // A review belongs to its generation/root, even if both repos have a.txt.
    send(
        &mut service,
        GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: roots[0].clone(),
            trusted: true,
        },
    );
    send(
        &mut service,
        GitCommand::Prepare(LocalAction::Stage {
            paths: vec!["b.txt".into()],
        }),
    );
    let preview = service.preview.clone().unwrap();
    send(
        &mut service,
        GitCommand::OpenWithin {
            workspace: dir.path().into(),
            path: roots[1].clone(),
            trusted: true,
        },
    );
    let before = git(&roots[1], &["write-tree"]);
    assert_eq!(
        service
            .dispatch(GitCommand::Apply {
                preview_id: preview.id,
                repo_generation: preview.generation
            })
            .unwrap_err()
            .kind,
        GitErrorKind::Stale
    );
    assert_eq!(git(&roots[1], &["write-tree"]), before);
}
