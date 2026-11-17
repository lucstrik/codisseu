use codisseu_git::{GitErrorKind, Repository, autoria::*};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
fn git(root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    dir
}
fn init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Alice Fixture"),
        ("user.email", "alice@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        git(root, &["config", key, value]);
    }
    let hooks = root.join(".git/empty-hooks");
    fs::create_dir(&hooks).unwrap();
    git(root, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
}
fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "--all"]);
    git(root, &["commit", "-m", message]);
    git(root, &["rev-parse", "HEAD"])
}
fn request(root: &Path, path: &str, contents: &str) -> Request {
    Request {
        workspace: root.into(),
        path: root.join(path),
        document_version: 7,
        contents: Arc::from(contents),
        start: 1,
        lines: 500,
    }
}
fn read(request: &Request) -> Snapshot {
    inspect(request, &AtomicBool::new(false)).unwrap()
}
#[test]
fn unchanged_and_unsaved_buffer_map_to_original_authors_without_writing() {
    let d = fixture();
    let path = d.path().join("hello.php");
    fs::write(&path, "first\nold second\n").unwrap();
    let alice = commit(d.path(), "Initial Alice");
    git(d.path(), &["config", "user.name", "Beatriz Fixture"]);
    git(d.path(), &["config", "user.email", "bea@example.invalid"]);
    fs::write(&path, "first\nsecond 🌧\n").unwrap();
    let bea = commit(d.path(), "Second Beatriz\n\nComplete body");
    let before = fs::read(&path).unwrap();
    let r = read(&request(
        d.path(),
        "hello.php",
        "inserted unsaved\nfirst\nsecond 🌧\n",
    ));
    assert_eq!(r.state, FileState::Tracked);
    assert_eq!(r.head.as_deref(), Some(bea.as_str()));
    assert_eq!(r.lines.len(), 4);
    assert!(r.lines[0].uncommitted);
    assert!(r.lines[0].author.is_empty());
    assert_eq!((&r.lines[1].oid, r.lines[1].original_line), (&alice, 1));
    assert_eq!(r.lines[1].author, "Alice Fixture");
    assert_eq!(r.lines[2].oid, bea);
    assert_eq!(r.lines[2].author, "Beatriz Fixture");
    assert_eq!(r.lines[2].summary, "Second Beatriz");
    assert_eq!(r.lines[2].text, "second 🌧".as_bytes());
    assert!(r.lines[2].previous_oid.is_some());
    assert!(r.lines[3].uncommitted); // virtual empty editor line
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(git(d.path(), &["status", "--porcelain"]), "");
}
#[test]
fn buffer_overrides_dirty_disk_and_cannot_inherit_wrong_line() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\ntwo\n").unwrap();
    let oid = commit(d.path(), "Original");
    fs::write(d.path().join("a"), "disk differs\none\ntwo\n").unwrap();
    let r = read(&request(d.path(), "a", "one\ntwo\n"));
    assert_eq!(r.lines[0].oid, oid);
    assert!(!r.lines[0].uncommitted);
    assert_eq!(r.lines[1].original_line, 2);
    assert_eq!(
        fs::read_to_string(d.path().join("a")).unwrap(),
        "disk differs\none\ntwo\n"
    );
}
#[test]
fn unicode_crlf_tabs_quotes_newline_and_leading_dash_paths_are_literal() {
    let d = fixture();
    let name = "- á \"tab\tnewline\n:hello.php";
    fs::write(d.path().join(name), "ação 🌧\r\n\tsecond\r\n").unwrap();
    let oid = commit(d.path(), "Caminhos próprios");
    let r = read(&request(d.path(), name, "ação 🌧\r\n\tsecond\r\n"));
    assert_eq!(r.lines[0].oid, oid);
    assert_eq!(r.lines[0].text, "ação 🌧\r".as_bytes());
    assert_eq!(r.lines[0].path, Path::new(name));
    assert_eq!(r.lines[1].text, b"\tsecond\r");
    assert_eq!(git(d.path(), &["status", "--porcelain"]), "");
}
#[test]
fn committed_and_staged_renames_keep_previous_authorship() {
    let d = fixture();
    fs::write(d.path().join("old.php"), "one\ntwo\n").unwrap();
    let oid = commit(d.path(), "Original name");
    git(d.path(), &["mv", "old.php", "new.php"]);
    let r = read(&request(d.path(), "new.php", "one\ntwo\n"));
    assert_eq!(r.state, FileState::Tracked);
    assert_eq!(r.lines[0].oid, oid);
    commit(d.path(), "Rename only");
    let r = read(&request(d.path(), "new.php", "one\ntwo\n"));
    assert_eq!(r.lines[0].oid, oid);
    assert_eq!(r.lines[0].path, Path::new("old.php"));
}
#[test]
fn untracked_added_unborn_and_empty_buffers_are_explicit() {
    let d = fixture();
    fs::write(d.path().join("new"), "new\n").unwrap();
    let r = read(&request(d.path(), "new", "new\n"));
    assert_eq!(r.state, FileState::Unborn);
    assert!(r.head.is_none());
    assert!(
        r.lines
            .iter()
            .all(|line| line.uncommitted && line.author.is_empty())
    );
    commit(d.path(), "First");
    fs::write(d.path().join("untracked"), "untracked").unwrap();
    assert_eq!(
        read(&request(d.path(), "untracked", "untracked")).state,
        FileState::Untracked
    );
    git(d.path(), &["add", "untracked"]);
    assert_eq!(
        read(&request(d.path(), "untracked", "untracked")).state,
        FileState::Added
    );
    let r = read(&request(d.path(), "new", ""));
    assert_eq!(r.lines.len(), 1);
    assert!(r.lines[0].uncommitted);
}
#[test]
fn removed_file_keeps_open_buffer_authorship() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    let oid = commit(d.path(), "Before removal");
    fs::remove_file(d.path().join("a")).unwrap();
    let r = read(&request(d.path(), "a", "one\n"));
    assert_eq!(r.lines[0].oid, oid);
    assert!(!d.path().join("a").exists());
}
#[test]
fn selected_workspace_directory_can_be_inside_repository() {
    let d = fixture();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::write(d.path().join("src/a"), "one\n").unwrap();
    commit(d.path(), "Initial");
    let r = read(&request(&d.path().join("src"), "a", "one\n"));
    assert_eq!(r.repository.root, d.path());
}
#[test]
fn nested_repository_is_discovered_independently_of_outer_scm_selection() {
    let d = fixture();
    fs::write(d.path().join("outer"), "outer\n").unwrap();
    commit(d.path(), "Outer");
    let child = d.path().join("child");
    init(&child);
    fs::write(child.join("inner"), "inner\n").unwrap();
    let oid = commit(&child, "Inner");
    let r = read(&request(d.path(), "child/inner", "inner\n"));
    assert_eq!(r.repository.root, child);
    assert_eq!(r.lines[0].oid, oid);
    let outer = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    assert_eq!(
        outer
            .blame_contents(
                Path::new("child/inner"),
                "inner\n",
                1,
                1,
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .kind,
        GitErrorKind::Invalid
    );
}
#[test]
fn traversal_outside_symlinks_and_git_internal_files_are_rejected() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    commit(d.path(), "First");
    for name in ["../escape", ".git/config", "sub/../a"] {
        assert_eq!(
            inspect(&request(d.path(), name, "one\n"), &AtomicBool::new(false))
                .unwrap_err()
                .kind,
            GitErrorKind::Invalid
        );
    }
    let outside = fixture();
    fs::write(outside.path().join("secret"), "secret\n").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), d.path().join("escape")).unwrap();
        assert_eq!(
            inspect(
                &request(d.path(), "escape/secret", "secret\n"),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Unsupported
        );
        std::os::unix::fs::symlink(outside.path().join("secret"), d.path().join("link")).unwrap();
        assert_eq!(
            inspect(
                &request(d.path(), "link", "secret\n"),
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Unsupported
        );
    }
}
#[test]
fn invalid_ranges_binary_and_size_budgets_are_rejected() {
    let d = fixture();
    let mut r = request(d.path(), "a", "one\n");
    r.start = 0;
    assert_eq!(
        inspect(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    r.start = 1;
    r.lines = 501;
    assert_eq!(
        inspect(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    r.lines = 1;
    r.contents = Arc::from("binary\0");
    assert_eq!(
        inspect(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Unsupported
    );
    r.contents = Arc::from("a".repeat(MAX_CONTENTS + 1));
    assert_eq!(
        inspect(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::OutputLimit
    );
    r.contents = Arc::from("\n".repeat(100_000));
    assert_eq!(
        inspect(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::OutputLimit
    );
}
#[test]
fn filtered_read_cannot_execute_clean_smudge_textconv_or_fsmonitor_helpers() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    commit(d.path(), "First");
    let marker = d.path().join("EXECUTED");
    let shell = format!("touch {}", marker.display());
    git(d.path(), &["config", "filter.fixture.clean", &shell]);
    git(d.path(), &["config", "filter.fixture.smudge", &shell]);
    git(d.path(), &["config", "filter.fixture.required", "true"]);
    git(d.path(), &["config", "diff.fixture.textconv", &shell]);
    git(d.path(), &["config", "core.fsmonitor", &shell]);
    fs::write(
        d.path().join(".gitattributes"),
        "a filter=fixture diff=fixture\n",
    )
    .unwrap();
    let r = read(&request(d.path(), "a", "one\n"));
    assert!(!r.lines[0].uncommitted);
    assert!(!marker.exists());
}
#[test]
fn worker_keeps_only_latest_request_and_cancel_discards_delivery() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    commit(d.path(), "First");
    let worker = Worker::new();
    let mut expected = 0;
    for version in 1..=80 {
        let mut r = request(d.path(), "a", "one\n");
        r.document_version = version;
        expected = worker.request(r).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    let reply = loop {
        if let Some(reply) = worker.poll() {
            break reply;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(reply.id, expected);
    assert_eq!(reply.document_version, 80);
    assert!(reply.result.is_ok());
    worker.request(request(d.path(), "a", "one\n")).unwrap();
    worker.cancel();
    while worker.is_busy() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(worker.poll().is_none());
}
#[test]
fn pre_cancelled_request_does_not_run_git() {
    let d = fixture();
    assert_eq!(
        inspect(&request(d.path(), "a", "one\n"), &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        GitErrorKind::Cancelled
    );
}
#[test]
fn line_history_follows_actual_changes_pins_head_and_limits_results() {
    let d = fixture();
    fs::write(d.path().join("a"), "first\nother\n").unwrap();
    let first = commit(d.path(), "First");
    fs::write(d.path().join("a"), "second\nother\n").unwrap();
    let second = commit(d.path(), "Second");
    fs::write(d.path().join("a"), "second\nelsewhere\n").unwrap();
    let tip = commit(d.path(), "Other line");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let history = repo
        .line_history(
            Path::new("a"),
            1,
            1,
            Some("second\nelsewhere\n"),
            100,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(history.root, d.path());
    assert_eq!(history.head, tip);
    assert_eq!(
        history.commits.iter().map(|c| &c.oid).collect::<Vec<_>>(),
        vec![&second, &first]
    );
    assert!(!history.has_more);
    let page = repo
        .line_history(Path::new("a"), 1, 1, None, 1, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(page.commits.len(), 1);
    assert!(page.has_more);
}
#[test]
fn line_history_refuses_dirty_buffer_and_dirty_disk_without_wrong_mapping() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\ntwo\n").unwrap();
    commit(d.path(), "First");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let error = repo
        .line_history(
            Path::new("a"),
            1,
            1,
            Some("new\none\ntwo\n"),
            10,
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Unsupported);
    assert!(error.message.contains("HEAD"));
    fs::write(d.path().join("a"), "new\none\ntwo\n").unwrap();
    assert_eq!(
        repo.line_history(Path::new("a"), 1, 1, None, 10, &AtomicBool::new(false))
            .unwrap_err()
            .kind,
        GitErrorKind::Unsupported
    );
    assert!(
        repo.line_history(
            Path::new("a"),
            1,
            1,
            Some("one\ntwo\n"),
            10,
            &AtomicBool::new(false)
        )
        .is_ok()
    );
}
#[test]
fn history_worker_preserves_document_identity_and_literal_path() {
    let d = fixture();
    let name = "-file:á.txt";
    fs::write(d.path().join(name), "one\n").unwrap();
    commit(d.path(), "First");
    let worker = HistoryWorker::new();
    let id = worker
        .request(HistoryRequest {
            workspace: d.path().into(),
            path: d.path().join(name),
            document_version: 11,
            contents: Arc::from("one\n"),
            start: 1,
            end: 1,
            limit: 10,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let reply = loop {
        if let Some(reply) = worker.poll() {
            break reply;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(reply.id, id);
    assert_eq!(reply.document_version, 11);
    assert_eq!(reply.path, d.path().join(name));
    let result = reply.result.unwrap();
    assert_eq!(result.path, Path::new(name));
    assert_eq!(result.commits.len(), 1);
}

#[test]
fn linked_worktree_keeps_its_own_git_identity() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    let oid = commit(d.path(), "Original");
    let destination = tempfile::tempdir().unwrap();
    let worktree = destination.path().join("linked");
    git(
        d.path(),
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            worktree.to_str().unwrap(),
        ],
    );
    let result = read(&request(&worktree, "a", "one\n"));
    assert_eq!(result.lines[0].oid, oid);
    assert_eq!(result.repository.root, worktree);
    assert!(result.repository.linked_worktree);
    assert_ne!(result.repository.git_dir, result.repository.common_dir);
}
#[test]
fn sha256_commit_identity_and_uncommitted_lines_are_supported() {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "--object-format=sha256", "-b", "main"]);
    for (key, value) in [
        ("user.name", "SHA256 Fixture"),
        ("user.email", "sha@example.invalid"),
        ("commit.gpgsign", "false"),
    ] {
        git(d.path(), &["config", key, value]);
    }
    fs::write(d.path().join("a"), "one\n").unwrap();
    let oid = commit(d.path(), "SHA256");
    assert_eq!(oid.len(), 64);
    let result = read(&request(d.path(), "a", "new\none\n"));
    assert!(result.lines[0].uncommitted);
    assert_eq!(result.lines[1].oid, oid);
}
#[test]
fn single_requested_virtual_line_never_reuses_previous_authorship() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    commit(d.path(), "One line");
    let mut r = request(d.path(), "a", "one\n");
    r.start = 2;
    r.lines = 1;
    let result = read(&r);
    assert_eq!(result.lines.len(), 1);
    assert_eq!(result.lines[0].line, 2);
    assert!(result.lines[0].uncommitted);
    assert!(result.lines[0].author.is_empty());
    r.contents = Arc::from("one");
    r.start = 1;
    let result = read(&r);
    assert_eq!(result.lines.len(), 1);
    assert!(result.lines[0].uncommitted); // removing the final newline changes this line
    fs::write(d.path().join("a"), "one").unwrap();
    let oid = commit(d.path(), "No final newline");
    let result = read(&r);
    assert!(!result.lines[0].uncommitted);
    assert_eq!(result.lines[0].oid, oid);
}
#[test]
fn line_history_follows_committed_file_rename() {
    let d = fixture();
    fs::write(d.path().join("old"), "one\ntwo\n").unwrap();
    let first = commit(d.path(), "Original name");
    git(d.path(), &["mv", "old", "new"]);
    commit(d.path(), "Rename");
    fs::write(d.path().join("new"), "changed\ntwo\n").unwrap();
    let changed = commit(d.path(), "Change selected line");
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let result = repo
        .line_history(
            Path::new("new"),
            1,
            1,
            Some("changed\ntwo\n"),
            10,
            &AtomicBool::new(false),
        )
        .unwrap();
    assert_eq!(result.commits.first().unwrap().oid, changed);
    assert_eq!(result.commits.last().unwrap().oid, first);
}
#[test]
fn conflicting_index_does_not_pretend_to_have_stable_authorship() {
    let d = fixture();
    fs::write(d.path().join("a"), "base\n").unwrap();
    commit(d.path(), "Base");
    git(d.path(), &["checkout", "-b", "other"]);
    fs::write(d.path().join("a"), "other\n").unwrap();
    commit(d.path(), "Other");
    git(d.path(), &["checkout", "main"]);
    fs::write(d.path().join("a"), "main\n").unwrap();
    commit(d.path(), "Main");
    let merge = Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["merge", "--no-edit", "other"])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let error = inspect(
        &request(d.path(), "a", "resolved but not staged\n"),
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert_eq!(error.kind, GitErrorKind::Unsupported);
    assert!(error.message.contains("conflitos"));
}

#[test]
fn cached_worker_replies_keep_each_request_id_and_document_version() {
    let d = fixture();
    fs::write(d.path().join("a"), "one\n").unwrap();
    let oid = commit(d.path(), "Cached worker");
    let worker = Worker::new();
    let mut prior_id = 0;
    for version in [7, 7, 8] {
        let mut r = request(d.path(), "a", "one\n");
        r.document_version = version;
        let expected = worker.request(r).unwrap();
        assert!(expected > prior_id);
        let deadline = Instant::now() + Duration::from_secs(20);
        let reply = loop {
            if let Some(reply) = worker.poll() {
                break reply;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(reply.id, expected);
        assert_eq!(reply.document_version, version);
        assert_eq!(reply.path, d.path().join("a"));
        assert_eq!(reply.result.unwrap().lines[0].oid, oid);
        prior_id = expected;
    }
}
