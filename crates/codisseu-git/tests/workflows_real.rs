//! Every semantic assertion runs installed Git against disposable local clones/bare remotes.
use codisseu_git::{Repository, conflicts::*, workflows::*};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

struct Fixture {
    _directory: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
    bare: PathBuf,
}
fn git(path: &Path, arguments: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn string(path: &Path, args: &[&str]) -> String {
    String::from_utf8(git(path, args)).unwrap().trim().into()
}
fn commit(path: &Path, message: &str) {
    git(path, &["add", "--all"]);
    git(path, &["commit", "-m", message]);
}
fn configure(path: &Path) {
    // The semantic fixtures can only contact their local bare transport even if
    // the desktop's global Git configuration contains URL rewrite rules.
    git(path, &["config", "protocol.allow", "never"]);
    git(path, &["config", "protocol.file.allow", "always"]);
    let hooks = path.join(".git/fixture-empty-hooks");
    fs::create_dir(&hooks).unwrap();
    git(path, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
    git(path, &["config", "user.name", "Codisseu fixture"]);
    git(path, &["config", "user.email", "fixture@example.test"]);
    git(path, &["config", "commit.gpgSign", "false"]);
    git(path, &["config", "core.autocrlf", "false"]);
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let bare = directory.path().join("remote.git");
        let a = directory.path().join("a");
        let b = directory.path().join("b");
        git(
            directory.path(),
            &[
                "init",
                "--bare",
                "--initial-branch=main",
                bare.to_str().unwrap(),
            ],
        );
        git(
            directory.path(),
            &["clone", bare.to_str().unwrap(), a.to_str().unwrap()],
        );
        configure(&a);
        fs::write(a.join("file.txt"), b"base\n").unwrap();
        commit(&a, "base");
        git(&a, &["push", "-u", "origin", "main"]);
        git(
            directory.path(),
            &["clone", bare.to_str().unwrap(), b.to_str().unwrap()],
        );
        configure(&b);
        Self {
            _directory: directory,
            a,
            b,
            bare,
        }
    }
    fn repo(&self) -> Repository {
        Repository::discover(&self.a, &AtomicBool::new(false)).unwrap()
    }
    fn divergence(&self) {
        git(&self.a, &["checkout", "-b", "topic"]);
        fs::write(self.a.join("file.txt"), b"topic\n").unwrap();
        commit(&self.a, "topic");
        git(&self.a, &["checkout", "main"]);
        fs::write(self.a.join("file.txt"), b"main\n").unwrap();
        commit(&self.a, "main");
    }
}
fn run(repo: &Repository, action: WorkflowAction) -> WorkflowReport {
    let cancel = AtomicBool::new(false);
    let plan = prepare_workflow(repo, action, &cancel).unwrap();
    apply_workflow(repo, &plan, &cancel).unwrap()
}

#[test]
fn merge_three_stages_atomic_result_explicit_stage_and_continue_are_real() {
    let f = Fixture::new();
    f.divergence();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    let before = string(&f.a, &["rev-parse", "HEAD"]);
    let plan = prepare_workflow(
        &repo,
        WorkflowAction::Merge {
            revision: "topic".into(),
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(before, string(&f.a, &["rev-parse", "HEAD"]));
    let report = apply_workflow(&repo, &plan, &cancel).unwrap();
    assert!(!report.success);
    assert_eq!(report.operation, Some(OperationKind::Merge));
    assert_eq!(report.conflicts, vec![PathBuf::from("file.txt")]);
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    assert_eq!(
        conflict.file.base.as_ref().unwrap().bytes.as_deref(),
        Some(b"base\n".as_slice())
    );
    assert_eq!(
        conflict.file.ours.as_ref().unwrap().bytes.as_deref(),
        Some(b"main\n".as_slice())
    );
    assert_eq!(
        conflict.file.theirs.as_ref().unwrap().bytes.as_deref(),
        Some(b"topic\n".as_slice())
    );
    let merge_model = conflict
        .file
        .text_merge
        .as_ref()
        .expect("worker model for a textual conflict");
    assert_eq!(merge_model.chunks.len(), 1);
    assert_eq!(
        merge_model.chunks[0].kind,
        codisseu_git::text_merge::MergeKind::Conflict
    );
    assert_eq!(merge_model.chunks[0].base_lines, 0..1);
    assert_eq!(
        merge_model.result(&[Some(codisseu_git::text_merge::MergeChoice::Both)]),
        "main\ntopic\n"
    );
    let resolution = prepare_resolution(
        &repo,
        &conflict,
        ResolutionChoice::EditedText("resolved 🦀\n".into()),
        false,
        &cancel,
    )
    .unwrap();
    let result = apply_resolution(&repo, &resolution, &cancel).unwrap();
    assert!(result.worktree_written && !result.staged);
    assert!(!git(&f.a, &["ls-files", "-u"]).is_empty());
    assert_eq!(
        fs::read_to_string(f.a.join("file.txt")).unwrap(),
        "resolved 🦀\n"
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    let resolution = prepare_resolution(
        &repo,
        &conflict,
        ResolutionChoice::EditedText("resolved 🦀\n".into()),
        true,
        &cancel,
    )
    .unwrap();
    assert!(
        apply_resolution(&repo, &resolution, &cancel)
            .unwrap()
            .staged
    );
    assert!(git(&f.a, &["ls-files", "-u"]).is_empty());
    assert!(run(&repo, WorkflowAction::Continue(OperationKind::Merge)).success);
    assert_eq!(
        git(&f.a, &["show", "HEAD:file.txt"]),
        "resolved 🦀\n".as_bytes()
    );
    assert_eq!(
        string(&f.a, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
}
#[test]
fn stale_conflict_or_workflow_approval_never_overwrites_new_disk_or_index() {
    let f = Fixture::new();
    f.divergence();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "topic".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    let plan = prepare_resolution(
        &repo,
        &conflict,
        ResolutionChoice::EditedText("obsolete\n".into()),
        true,
        &cancel,
    )
    .unwrap();
    fs::write(f.a.join("file.txt"), b"external change\n").unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).is_err());
    assert_eq!(
        fs::read(f.a.join("file.txt")).unwrap(),
        b"external change\n"
    );
    assert!(run(&repo, WorkflowAction::Abort(OperationKind::Merge)).success);
    let plan = prepare_workflow(
        &repo,
        WorkflowAction::Reset {
            revision: "HEAD~1".into(),
            mode: ResetMode::Hard,
        },
        &cancel,
    )
    .unwrap();
    assert!(plan.preview.destructive);
    let head = string(&f.a, &["rev-parse", "HEAD"]);
    fs::write(f.a.join("new.txt"), b"external").unwrap();
    git(&f.a, &["add", "new.txt"]);
    assert!(apply_workflow(&repo, &plan, &cancel).is_err());
    assert_eq!(head, string(&f.a, &["rev-parse", "HEAD"]));
    assert_eq!(git(&f.a, &["show", ":new.txt"]), b"external");
}
#[test]
fn rebase_conflict_side_semantics_continue_and_abort_preserve_real_history() {
    let f = Fixture::new();
    f.divergence();
    git(&f.a, &["checkout", "topic"]);
    let old = string(&f.a, &["rev-parse", "HEAD"]);
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    assert!(
        !run(
            &repo,
            WorkflowAction::Rebase {
                onto: "main".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    assert_eq!(
        conflict.file.ours.as_ref().unwrap().bytes.as_deref(),
        Some(b"main\n".as_slice())
    );
    assert_eq!(
        conflict.file.theirs.as_ref().unwrap().bytes.as_deref(),
        Some(b"topic\n".as_slice())
    );
    assert!(
        conflict
            .file
            .warnings
            .iter()
            .any(|w| w.contains("Durante rebase"))
    );
    assert!(run(&repo, WorkflowAction::Abort(OperationKind::Rebase)).success);
    assert_eq!(string(&f.a, &["rev-parse", "HEAD"]), old);
    assert_eq!(git(&f.a, &["show", "HEAD:file.txt"]), b"topic\n");
    assert!(
        !run(
            &repo,
            WorkflowAction::Rebase {
                onto: "main".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    let plan = prepare_resolution(
        &repo,
        &conflict,
        ResolutionChoice::EditedText("rebased\n".into()),
        true,
        &cancel,
    )
    .unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).unwrap().staged);
    assert!(run(&repo, WorkflowAction::Continue(OperationKind::Rebase)).success);
    assert_eq!(string(&f.a, &["branch", "--show-current"]), "topic");
    assert_eq!(git(&f.a, &["show", "HEAD:file.txt"]), b"rebased\n");
    assert_eq!(string(&f.a, &["rev-list", "--count", "main..topic"]), "1");
}
#[test]
fn add_add_and_modify_delete_require_explicit_side_or_deletion() {
    let f = Fixture::new();
    let cancel = AtomicBool::new(false);
    git(&f.a, &["checkout", "-b", "topic"]);
    fs::write(f.a.join("added.txt"), b"topic").unwrap();
    commit(&f.a, "topic add");
    git(&f.a, &["checkout", "main"]);
    fs::write(f.a.join("added.txt"), b"main").unwrap();
    commit(&f.a, "main add");
    let repo = f.repo();
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "topic".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("added.txt"), &cancel).unwrap();
    assert!(conflict.file.base.is_none() && conflict.file.editable);
    let plan =
        prepare_resolution(&repo, &conflict, ResolutionChoice::Theirs, true, &cancel).unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).unwrap().staged);
    assert_eq!(git(&f.a, &["show", ":added.txt"]), b"topic");
    assert!(run(&repo, WorkflowAction::Abort(OperationKind::Merge)).success);
    git(&f.a, &["checkout", "-b", "deletion"]);
    git(&f.a, &["rm", "file.txt"]);
    commit(&f.a, "delete file");
    git(&f.a, &["checkout", "main"]);
    fs::write(f.a.join("file.txt"), b"modified").unwrap();
    commit(&f.a, "modify file");
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "deletion".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    assert!(conflict.file.theirs.is_none());
    assert!(prepare_resolution(&repo, &conflict, ResolutionChoice::Theirs, true, &cancel).is_err());
    let plan =
        prepare_resolution(&repo, &conflict, ResolutionChoice::Delete, true, &cancel).unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).unwrap().staged);
    assert!(!f.a.join("file.txt").exists());
    assert!(git(&f.a, &["ls-files", "-u"]).is_empty());
    assert!(string(&f.a, &["diff", "--cached", "--name-status"]).starts_with("D\tfile.txt"));
}
#[test]
fn binary_side_resolution_keeps_exact_bytes_and_never_opens_text_editor() {
    let f = Fixture::new();
    fs::write(f.a.join("binary.dat"), b"\0base").unwrap();
    commit(&f.a, "binary base");
    git(&f.a, &["checkout", "-b", "binary-topic"]);
    fs::write(f.a.join("binary.dat"), b"\0theirs\xff").unwrap();
    commit(&f.a, "binary theirs");
    git(&f.a, &["checkout", "main"]);
    fs::write(f.a.join("binary.dat"), b"\0ours\xfe").unwrap();
    commit(&f.a, "binary ours");
    let repo = f.repo();
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "binary-topic".into()
            }
        )
        .success
    );
    let cancel = AtomicBool::new(false);
    let conflict = load_conflict(&repo, Path::new("binary.dat"), &cancel).unwrap();
    assert!(!conflict.file.editable);
    assert!(
        prepare_resolution(
            &repo,
            &conflict,
            ResolutionChoice::EditedText("fake".into()),
            true,
            &cancel
        )
        .is_err()
    );
    let plan =
        prepare_resolution(&repo, &conflict, ResolutionChoice::Theirs, true, &cancel).unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).unwrap().staged);
    assert_eq!(git(&f.a, &["show", ":binary.dat"]), b"\0theirs\xff");
    assert_eq!(fs::read(f.a.join("binary.dat")).unwrap(), b"\0theirs\xff");
}
#[cfg(unix)]
#[test]
fn symlink_side_preserves_link_mode_and_parent_escape_is_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink("base", f.a.join("link")).unwrap();
    commit(&f.a, "link base");
    git(&f.a, &["checkout", "-b", "link-topic"]);
    fs::remove_file(f.a.join("link")).unwrap();
    symlink("../outside", f.a.join("link")).unwrap();
    commit(&f.a, "link theirs");
    git(&f.a, &["checkout", "main"]);
    fs::remove_file(f.a.join("link")).unwrap();
    symlink("ours", f.a.join("link")).unwrap();
    commit(&f.a, "link ours");
    fs::write(f._directory.path().join("outside"), b"sentinel").unwrap();
    let repo = f.repo();
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "link-topic".into()
            }
        )
        .success
    );
    let cancel = AtomicBool::new(false);
    let conflict = load_conflict(&repo, Path::new("link"), &cancel).unwrap();
    assert!(!conflict.file.editable);
    let plan =
        prepare_resolution(&repo, &conflict, ResolutionChoice::Theirs, true, &cancel).unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).unwrap().staged);
    assert_eq!(
        fs::read_link(f.a.join("link")).unwrap(),
        Path::new("../outside")
    );
    assert!(string(&f.a, &["ls-files", "--stage", "link"]).starts_with("120000"));
    assert_eq!(
        fs::read(f._directory.path().join("outside")).unwrap(),
        b"sentinel"
    );
    assert!(load_conflict(&repo, Path::new("../outside"), &cancel).is_err());
    assert!(load_conflict(&repo, Path::new(".git/config"), &cancel).is_err());
}
#[test]
fn stash_lists_apply_pop_drop_and_stale_reference_do_not_discard_wrong_entry() {
    let f = Fixture::new();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    fs::write(f.a.join("file.txt"), b"WIP").unwrap();
    fs::write(f.a.join("untracked.txt"), b"untracked").unwrap();
    assert!(
        run(
            &repo,
            WorkflowAction::StashCreate {
                message: "tracked only".into(),
                include_untracked: false
            }
        )
        .success
    );
    assert_eq!(fs::read(f.a.join("file.txt")).unwrap(), b"base\n");
    assert!(f.a.join("untracked.txt").exists());
    let original = list_stashes(&repo, &cancel).unwrap()[0].oid.clone();
    assert!(
        run(
            &repo,
            WorkflowAction::StashCreate {
                message: "including untracked".into(),
                include_untracked: true
            }
        )
        .success
    );
    assert!(!f.a.join("untracked.txt").exists());
    assert_eq!(list_stashes(&repo, &cancel).unwrap().len(), 2);
    assert!(
        run(
            &repo,
            WorkflowAction::StashApply {
                stash: "stash@{1}".into(),
                pop: true
            }
        )
        .success
    );
    assert_eq!(fs::read(f.a.join("file.txt")).unwrap(), b"WIP");
    assert_eq!(list_stashes(&repo, &cancel).unwrap().len(), 1);
    assert!(
        !list_stashes(&repo, &cancel)
            .unwrap()
            .iter()
            .any(|s| s.oid == original)
    );
    git(&f.a, &["restore", "file.txt"]);
    let plan = prepare_workflow(
        &repo,
        WorkflowAction::StashDrop {
            stash: "stash@{0}".into(),
        },
        &cancel,
    )
    .unwrap();
    let preserved = list_stashes(&repo, &cancel).unwrap()[0].oid.clone();
    fs::write(f.a.join("file.txt"), b"new stash").unwrap();
    git(&f.a, &["stash", "push", "-m", "external stash"]);
    assert!(apply_workflow(&repo, &plan, &cancel).is_err());
    assert!(
        list_stashes(&repo, &cancel)
            .unwrap()
            .iter()
            .any(|s| s.oid == preserved)
    );
    assert!(
        run(
            &repo,
            WorkflowAction::StashDrop {
                stash: "stash@{1}".into()
            }
        )
        .success
    );
    assert_eq!(list_stashes(&repo, &cancel).unwrap().len(), 1);
}
#[test]
fn stash_pop_conflict_keeps_stash_until_user_resolves_it() {
    let f = Fixture::new();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    fs::write(f.a.join("file.txt"), b"stash version\n").unwrap();
    assert!(
        run(
            &repo,
            WorkflowAction::StashCreate {
                message: "conflict".into(),
                include_untracked: false
            }
        )
        .success
    );
    let saved = list_stashes(&repo, &cancel).unwrap()[0].oid.clone();
    fs::write(f.a.join("file.txt"), b"committed version\n").unwrap();
    commit(&f.a, "conflicting commit");
    let result = run(
        &repo,
        WorkflowAction::StashApply {
            stash: "stash@{0}".into(),
            pop: true,
        },
    );
    assert!(!result.success);
    assert_eq!(result.conflicts, vec![PathBuf::from("file.txt")]);
    assert_eq!(list_stashes(&repo, &cancel).unwrap()[0].oid, saved);
}
#[test]
fn local_bare_fetch_pull_push_and_non_fast_forward_preserve_remote_commits() {
    let f = Fixture::new();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    let other = Repository::discover(&f.b, &cancel).unwrap();
    fs::write(f.b.join("file.txt"), b"from b\n").unwrap();
    commit(&f.b, "b commit");
    assert!(
        run(
            &other,
            WorkflowAction::Push {
                remote: "origin".into(),
                branch: "main".into()
            }
        )
        .success
    );
    assert_eq!(git(&f.bare, &["show", "main:file.txt"]), b"from b\n");
    let local = string(&f.a, &["rev-parse", "HEAD"]);
    assert!(
        run(
            &repo,
            WorkflowAction::Fetch {
                remote: "origin".into()
            }
        )
        .success
    );
    assert_eq!(string(&f.a, &["rev-parse", "HEAD"]), local);
    assert!(
        run(
            &repo,
            WorkflowAction::Pull {
                remote: "origin".into(),
                branch: "main".into(),
                mode: PullMode::FastForwardOnly
            }
        )
        .success
    );
    assert_eq!(fs::read(f.a.join("file.txt")).unwrap(), b"from b\n");
    fs::write(f.a.join("a.txt"), b"a").unwrap();
    commit(&f.a, "a unique");
    assert!(
        run(
            &repo,
            WorkflowAction::Push {
                remote: "origin".into(),
                branch: "main".into()
            }
        )
        .success
    );
    let remote_before = string(&f.bare, &["rev-parse", "main"]);
    fs::write(f.b.join("b.txt"), b"b").unwrap();
    commit(&f.b, "b unique");
    assert!(
        !run(
            &other,
            WorkflowAction::Push {
                remote: "origin".into(),
                branch: "main".into()
            }
        )
        .success
    );
    assert_eq!(string(&f.bare, &["rev-parse", "main"]), remote_before);
    assert!(
        !run(
            &other,
            WorkflowAction::Pull {
                remote: "origin".into(),
                branch: "main".into(),
                mode: PullMode::FastForwardOnly
            }
        )
        .success
    );
    assert!(
        run(
            &other,
            WorkflowAction::Pull {
                remote: "origin".into(),
                branch: "main".into(),
                mode: PullMode::Merge
            }
        )
        .success
    );
    assert!(f.b.join("a.txt").exists() && f.b.join("b.txt").exists());
    assert_eq!(
        string(&f.b, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
}
#[test]
fn cherry_pick_revert_and_explicit_reset_modes_have_independent_disk_effects() {
    let f = Fixture::new();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    git(&f.a, &["checkout", "-b", "patch"]);
    fs::write(f.a.join("patch.txt"), b"patch").unwrap();
    commit(&f.a, "patch");
    git(&f.a, &["checkout", "main"]);
    assert!(
        run(
            &repo,
            WorkflowAction::CherryPick {
                revision: "patch".into()
            }
        )
        .success
    );
    let picked = string(&f.a, &["rev-parse", "HEAD"]);
    assert_eq!(fs::read(f.a.join("patch.txt")).unwrap(), b"patch");
    assert!(
        run(
            &repo,
            WorkflowAction::Revert {
                revision: "HEAD".into()
            }
        )
        .success
    );
    assert!(!f.a.join("patch.txt").exists());
    assert!(
        run(
            &repo,
            WorkflowAction::Reset {
                revision: "HEAD~1".into(),
                mode: ResetMode::Soft
            }
        )
        .success
    );
    assert!(string(&f.a, &["diff", "--cached", "--name-status"]).contains("D\tpatch.txt"));
    assert!(
        run(
            &repo,
            WorkflowAction::Reset {
                revision: "HEAD~1".into(),
                mode: ResetMode::Mixed
            }
        )
        .success
    );
    assert!(git(&f.a, &["diff", "--cached"]).is_empty());
    assert!(
        run(
            &repo,
            WorkflowAction::Reset {
                revision: picked,
                mode: ResetMode::Hard
            }
        )
        .success
    );
    assert_eq!(fs::read(f.a.join("patch.txt")).unwrap(), b"patch");
    assert!(
        prepare_workflow(
            &repo,
            WorkflowAction::Merge {
                revision: "--upload-pack=evil".into()
            },
            &cancel
        )
        .is_err()
    );
    assert!(
        prepare_workflow(
            &repo,
            WorkflowAction::Push {
                remote: "--evil".into(),
                branch: "main".into()
            },
            &cancel
        )
        .is_err()
    );
    assert!(
        prepare_workflow(
            &repo,
            WorkflowAction::Push {
                remote: "origin".into(),
                branch: "main:other".into()
            },
            &cancel
        )
        .is_err()
    );
}
#[test]
fn remote_preview_binds_configured_url_and_public_lists_redact_credentials() {
    let f = Fixture::new();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    let plan = prepare_workflow(
        &repo,
        WorkflowAction::Push {
            remote: "origin".into(),
            branch: "main".into(),
        },
        &cancel,
    )
    .unwrap();
    let changed = f._directory.path().join("other.git");
    git(
        f._directory.path(),
        &["init", "--bare", changed.to_str().unwrap()],
    );
    git(
        &f.a,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            changed.to_str().unwrap(),
        ],
    );
    assert!(apply_workflow(&repo, &plan, &cancel).is_err());
    git(
        &f.a,
        &[
            "remote",
            "add",
            "fake-auth",
            "https://reader:fake-secret@fixture.invalid/repo.git?token=fake-query",
        ],
    );
    let remotes = list_remotes(&repo, &cancel).unwrap();
    let auth = remotes.iter().find(|r| r.name == "fake-auth").unwrap();
    assert!(!auth.fetch_url.contains("fake-secret"));
    assert!(!auth.push_url.contains("fake-secret"));
    assert!(!auth.fetch_url.contains("fake-query"));
}

#[test]
fn failed_index_stage_reports_saved_result_without_claiming_resolution_or_rollback() {
    let f = Fixture::new();
    f.divergence();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "topic".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    let plan = prepare_resolution(
        &repo,
        &conflict,
        ResolutionChoice::EditedText("saved but not staged\n".into()),
        true,
        &cancel,
    )
    .unwrap();
    let lock = f.a.join(".git/index.lock");
    fs::write(&lock, b"owned test lock").unwrap();
    let result = apply_resolution(&repo, &plan, &cancel).unwrap();
    assert!(result.worktree_written && !result.staged && result.error.is_some());
    assert_eq!(
        fs::read(f.a.join("file.txt")).unwrap(),
        b"saved but not staged\n"
    );
    assert!(!git(&f.a, &["ls-files", "-u"]).is_empty());
    fs::remove_file(lock).unwrap();
}

#[test]
fn display_metadata_cannot_reauthorize_another_resolution_or_workflow() {
    let f = Fixture::new();
    f.divergence();
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    let mut workflow = prepare_workflow(
        &repo,
        WorkflowAction::Merge {
            revision: "topic".into(),
        },
        &cancel,
    )
    .unwrap();
    workflow.preview.action = WorkflowAction::Reset {
        revision: "topic".into(),
        mode: ResetMode::Hard,
    };
    assert!(apply_workflow(&repo, &workflow, &cancel).is_err());
    assert_eq!(string(&f.a, &["branch", "--show-current"]), "main");
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "topic".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    let mut altered = conflict.clone();
    altered.file.path = "other.txt".into();
    assert!(prepare_resolution(&repo, &altered, ResolutionChoice::Ours, true, &cancel).is_err());
    let mut plan =
        prepare_resolution(&repo, &conflict, ResolutionChoice::Ours, false, &cancel).unwrap();
    plan.preview.stage = true;
    assert!(apply_resolution(&repo, &plan, &cancel).is_err());
    assert!(!git(&f.a, &["ls-files", "-u"]).is_empty());
}

#[cfg(unix)]
#[test]
fn replacing_conflict_parent_by_symlink_never_writes_the_external_file() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::create_dir(f.a.join("nested")).unwrap();
    fs::write(f.a.join("nested/file.txt"), b"nested base").unwrap();
    commit(&f.a, "nested base");
    git(&f.a, &["checkout", "-b", "nested-topic"]);
    fs::write(f.a.join("nested/file.txt"), b"nested topic").unwrap();
    commit(&f.a, "nested topic");
    git(&f.a, &["checkout", "main"]);
    fs::write(f.a.join("nested/file.txt"), b"nested main").unwrap();
    commit(&f.a, "nested main");
    let repo = f.repo();
    let cancel = AtomicBool::new(false);
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "nested-topic".into()
            }
        )
        .success
    );
    let conflict = load_conflict(&repo, Path::new("nested/file.txt"), &cancel).unwrap();
    let plan = prepare_resolution(
        &repo,
        &conflict,
        ResolutionChoice::EditedText("must not escape".into()),
        true,
        &cancel,
    )
    .unwrap();
    let outside = f._directory.path().join("outside-directory");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("file.txt"), b"external sentinel").unwrap();
    fs::rename(f.a.join("nested"), f.a.join("original-nested")).unwrap();
    symlink(&outside, f.a.join("nested")).unwrap();
    assert!(load_conflict(&repo, Path::new("nested/file.txt"), &cancel).is_err());
    assert!(apply_resolution(&repo, &plan, &cancel).is_err());
    assert_eq!(
        fs::read(outside.join("file.txt")).unwrap(),
        b"external sentinel"
    );
}

#[test]
fn separated_rename_conflict_is_visible_and_never_resolved_as_unrelated_add_delete() {
    let f = Fixture::new();
    let cancel = AtomicBool::new(false);
    git(&f.a, &["checkout", "-b", "rename-topic"]);
    git(&f.a, &["mv", "file.txt", "theirs-name.txt"]);
    commit(&f.a, "rename theirs");
    git(&f.a, &["checkout", "main"]);
    git(&f.a, &["mv", "file.txt", "ours-name.txt"]);
    commit(&f.a, "rename ours");
    let repo = f.repo();
    let report = run(
        &repo,
        WorkflowAction::Merge {
            revision: "rename-topic".into(),
        },
    );
    assert!(!report.success);
    assert_eq!(report.conflicts.len(), 3);
    let before = git(&f.a, &["ls-files", "-u"]);
    let conflict = load_conflict(&repo, Path::new("ours-name.txt"), &cancel).unwrap();
    assert!(
        conflict
            .file
            .unsupported_reason
            .as_ref()
            .unwrap()
            .contains("renomes")
    );
    assert!(!conflict.file.editable);
    assert!(prepare_resolution(&repo, &conflict, ResolutionChoice::Ours, true, &cancel).is_err());
    assert_eq!(git(&f.a, &["ls-files", "-u"]), before);
}

#[cfg(unix)]
#[test]
fn accepting_executable_side_preserves_selected_index_and_disk_mode() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    git(&f.a, &["checkout", "-b", "executable-topic"]);
    fs::write(f.a.join("file.txt"), b"theirs executable\n").unwrap();
    fs::set_permissions(f.a.join("file.txt"), fs::Permissions::from_mode(0o755)).unwrap();
    commit(&f.a, "executable theirs");
    git(&f.a, &["checkout", "main"]);
    fs::write(f.a.join("file.txt"), b"ours regular\n").unwrap();
    commit(&f.a, "regular ours");
    let repo = f.repo();
    assert!(
        !run(
            &repo,
            WorkflowAction::Merge {
                revision: "executable-topic".into()
            }
        )
        .success
    );
    let cancel = AtomicBool::new(false);
    let conflict = load_conflict(&repo, Path::new("file.txt"), &cancel).unwrap();
    assert_eq!(conflict.file.theirs.as_ref().unwrap().mode, "100755");
    let plan =
        prepare_resolution(&repo, &conflict, ResolutionChoice::Theirs, true, &cancel).unwrap();
    assert!(apply_resolution(&repo, &plan, &cancel).unwrap().staged);
    assert_ne!(
        fs::metadata(f.a.join("file.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(string(&f.a, &["ls-files", "--stage", "file.txt"]).starts_with("100755"));
}
