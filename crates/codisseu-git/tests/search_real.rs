use codisseu_git::{GitErrorKind, Repository, search::*};
use std::{
    fs,
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn init(root: &Path) {
    fs::create_dir_all(root).unwrap();
    git(root, &["init", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Alice.* Fixture"),
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
fn fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    init(d.path());
    d
}
fn commit(root: &Path, path: &str, text: &str, message: &str) -> String {
    fs::write(root.join(path), text).unwrap();
    git(root, &["add", "--all"]);
    git(root, &["commit", "-m", message]);
    git(root, &["rev-parse", "HEAD"])
}
fn request(root: &Path) -> Request {
    Request {
        workspace: root.into(),
        root: root.into(),
        query: String::new(),
        author: String::new(),
        path: None,
        reference: None,
        skip: 0,
        limit: 200,
    }
}
fn run(request: &Request) -> SearchPage {
    search(request, &AtomicBool::new(false)).unwrap()
}
#[test]
fn query_is_literal_matches_body_and_ands_with_literal_author_or_email() {
    let d = fixture();
    let literal = commit(
        d.path(),
        "a",
        "one",
        "Literal .* [x]\n\nBody NEEDLE only here",
    );
    git(d.path(), &["config", "user.name", "AliceXYZ Fixture"]);
    commit(d.path(), "a", "two", "Regex-like query [x] alone");
    git(d.path(), &["config", "user.name", "Beatriz Fixture"]);
    git(d.path(), &["config", "user.email", "bea@example.invalid"]);
    commit(d.path(), "a", "three", "Body needle by another author");
    let mut r = request(d.path());
    r.query = ".* [x]".into();
    assert_eq!(run(&r).commits.len(), 1);
    r.query = "body needle".into();
    r.author = "ALICE.*".into();
    let result = run(&r);
    assert_eq!(result.commits.len(), 1);
    assert_eq!(result.commits[0].oid, literal);
    r.author = "bea@example.invalid".into();
    assert_eq!(run(&r).commits.len(), 1);
    r.query = "does not exist".into();
    assert!(run(&r).commits.is_empty());
    r.query.clear();
    r.author = "Alice.*".into();
    assert_eq!(run(&r).commits.len(), 1);
    let stamp = git(d.path(), &["show", "-s", "--format=%at"]);
    r.author = stamp;
    assert!(run(&r).commits.is_empty());
}
#[test]
fn pagination_filters_before_skip_and_pins_an_oid_when_head_moves() {
    let d = fixture();
    let mut matching = Vec::new();
    for n in 0..7 {
        let oid = commit(
            d.path(),
            "a",
            &format!("{n}\n"),
            if n % 2 == 0 { "Match item" } else { "Excluded" },
        );
        if n % 2 == 0 {
            matching.push(oid);
        }
    }
    let mut r = request(d.path());
    r.query = "match".into();
    r.limit = 2;
    let first = run(&r);
    assert!(first.has_more);
    assert_eq!(first.commits.len(), 2);
    assert_eq!(first.commits[0].oid, matching[3]);
    assert_eq!(first.commits[1].oid, matching[2]);
    let pinned = first.resolved_reference.clone().unwrap();
    commit(d.path(), "a", "new tip", "Match after first page");
    r.reference = Some(pinned.clone());
    r.skip = 2;
    let second = run(&r);
    assert!(!second.has_more);
    assert_eq!(second.resolved_reference.as_deref(), Some(pinned.as_str()));
    assert_eq!(second.commits[0].oid, matching[1]);
    assert_eq!(second.commits[1].oid, matching[0]);
    r.skip = 4;
    assert!(run(&r).commits.is_empty());
}
#[test]
fn explicit_branch_tag_and_oid_can_search_beyond_head() {
    let d = fixture();
    let base = commit(d.path(), "a", "base", "Base");
    git(d.path(), &["checkout", "-b", "feature"]);
    let feature = commit(d.path(), "a", "feature", "Only feature branch");
    git(d.path(), &["tag", "release"]);
    git(d.path(), &["checkout", "main"]);
    let mut r = request(d.path());
    r.query = "Only feature".into();
    assert!(run(&r).commits.is_empty());
    for reference in ["refs/heads/feature", "release", feature.as_str()] {
        r.reference = Some(reference.into());
        let result = run(&r);
        assert_eq!(result.commits.len(), 1);
        assert_eq!(result.commits[0].oid, feature);
    }
    r.query.clear();
    r.reference = Some(base.clone());
    assert_eq!(run(&r).commits[0].oid, base);
    r.reference = Some("does-not-exist".into());
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Git
    );
}
#[test]
fn path_filter_supports_directories_and_treats_globs_colons_and_leading_dash_literally() {
    let d = fixture();
    fs::create_dir(d.path().join("src")).unwrap();
    fs::create_dir(d.path().join("other")).unwrap();
    let exact = commit(d.path(), "src/-[x]:á.php", "one", "Exact path");
    commit(d.path(), "src/ordinary.php", "two", "Other source");
    commit(d.path(), "other/a.php", "three", "Elsewhere");
    let mut r = request(d.path());
    r.path = Some("src/-[x]:á.php".into());
    let result = run(&r);
    assert_eq!(result.commits.len(), 1);
    assert_eq!(result.commits[0].oid, exact);
    r.path = Some("src".into());
    assert_eq!(run(&r).commits.len(), 2);
    r.path = Some(d.path().join("src"));
    assert_eq!(run(&r).commits.len(), 2);
    r.path = Some(".".into());
    assert_eq!(run(&r).commits.len(), 3);
    r.path = Some("src/*.php".into());
    assert!(run(&r).commits.is_empty());
    fs::remove_file(d.path().join("src/-[x]:á.php")).unwrap();
    git(d.path(), &["add", "--all"]);
    git(d.path(), &["commit", "-m", "Delete"]);
    r.path = Some("src/-[x]:á.php".into());
    assert_eq!(run(&r).commits.len(), 2);
}
#[test]
fn nested_repository_roots_are_resolved_without_accepting_external_roots() {
    let outer = fixture();
    commit(outer.path(), "outer", "outer", "Outer");
    let child = outer.path().join("child");
    init(&child);
    let oid = commit(&child, "inner", "inner", "Inner");
    let mut r = request(&child);
    r.workspace = outer.path().into();
    let result = run(&r);
    assert_eq!(result.root, child);
    assert_eq!(result.commits[0].oid, oid);
    let external = fixture();
    r.root = external.path().into();
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    r.workspace = child.clone();
    r.root = outer.path().into();
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    let repo = Repository::discover(outer.path(), &AtomicBool::new(false)).unwrap();
    r.root = child;
    assert_eq!(
        repo.search_history(&r, &AtomicBool::new(false))
            .unwrap_err()
            .kind,
        GitErrorKind::Invalid
    );
}
#[test]
fn child_workspace_inside_repository_can_search_its_actual_root() {
    let d = fixture();
    fs::create_dir(d.path().join("src")).unwrap();
    commit(d.path(), "src/a", "one", "Source");
    let mut r = request(d.path());
    r.workspace = d.path().join("src");
    assert_eq!(run(&r).root, d.path());
}
#[test]
fn path_escape_git_internal_and_symlink_filters_are_rejected() {
    let d = fixture();
    commit(d.path(), "a", "one", "One");
    let mut r = request(d.path());
    for path in ["../other", ".git/config", "src/../a"] {
        r.path = Some(path.into());
        assert_eq!(
            search(&r, &AtomicBool::new(false)).unwrap_err().kind,
            GitErrorKind::Invalid
        );
    }
    let outside = fixture();
    r.path = Some(outside.path().join("a"));
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), d.path().join("escape")).unwrap();
        r.path = Some("escape/a".into());
        assert_eq!(
            search(&r, &AtomicBool::new(false)).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        r.root = d.path().join("escape");
        r.path = None;
        assert_eq!(
            search(&r, &AtomicBool::new(false)).unwrap_err().kind,
            GitErrorKind::Invalid
        );
    }
}
#[test]
fn bounds_and_option_like_values_never_become_git_options_or_shell_commands() {
    let d = fixture();
    let marker = d.path().join("SHOULD_NOT_EXIST");
    let mut r = request(d.path());
    r.query = format!("--all $(touch {})", marker.display());
    r.author = "--all".into();
    assert!(run(&r).commits.is_empty());
    assert!(!marker.exists());
    r.reference = Some("--all".into());
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    r.reference = None;
    for (skip, limit) in [(5001, 1), (0, 201), (0, 0)] {
        r.skip = skip;
        r.limit = limit;
        assert_eq!(
            search(&r, &AtomicBool::new(false)).unwrap_err().kind,
            GitErrorKind::Invalid
        );
    }
    r.skip = 0;
    r.limit = 1;
    r.query = "a".repeat(4097);
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    r.query = "two\npatterns".into();
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
    r.query.clear();
    r.author = "a\0b".into();
    assert_eq!(
        search(&r, &AtomicBool::new(false)).unwrap_err().kind,
        GitErrorKind::Invalid
    );
}
#[test]
fn worker_coalesces_requests_cancels_and_preserves_filter_identity() {
    let d = fixture();
    commit(d.path(), "a", "one", "One");
    let worker = Worker::new();
    let mut expected = 0;
    let mut final_request = request(d.path());
    for index in 0..50 {
        let mut r = request(d.path());
        r.query = if index == 49 {
            "One".into()
        } else {
            format!("superseded{index}")
        };
        expected = worker.request(r.clone()).unwrap();
        final_request = r;
    }
    let until = Instant::now() + Duration::from_secs(20);
    let reply = loop {
        if let Some(r) = worker.poll() {
            break r;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(reply.id, expected);
    assert_eq!(reply.request, final_request);
    assert_eq!(reply.result.unwrap().commits.len(), 1);
    worker.request(request(d.path())).unwrap();
    worker.cancel();
    while worker.is_busy() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(worker.poll().is_none());
    assert_eq!(
        search(&request(d.path()), &AtomicBool::new(true))
            .unwrap_err()
            .kind,
        GitErrorKind::Cancelled
    );
}
#[test]
fn unborn_and_zero_matches_are_successful_empty_pages_with_real_root() {
    let d = fixture();
    let page = run(&request(d.path()));
    assert_eq!(page.root, d.path());
    assert!(page.commits.is_empty());
    assert!(!page.has_more);
    assert!(page.resolved_reference.is_none());
    commit(d.path(), "a", "one", "One");
    let mut r = request(d.path());
    r.query = "No such message".into();
    let page = run(&r);
    assert!(page.commits.is_empty());
    assert!(!page.has_more);
    assert!(page.resolved_reference.is_some());
}
#[test]
fn returned_metadata_includes_unicode_author_email_parents_and_refs() {
    let d = fixture();
    let first = commit(d.path(), "a", "one", "First");
    git(d.path(), &["config", "user.name", "Bárbara 🌧"]);
    git(
        d.path(),
        &["config", "user.email", "barbara@example.invalid"],
    );
    let second = commit(d.path(), "a", "two", "Ação própria");
    git(d.path(), &["tag", "release"]);
    let page = run(&request(d.path()));
    let entry = &page.commits[0];
    assert_eq!(entry.oid, second);
    assert_eq!(entry.parents, vec![first]);
    assert_eq!(entry.author, "Bárbara 🌧");
    assert_eq!(entry.email, "barbara@example.invalid");
    assert_eq!(entry.subject, "Ação própria");
    assert!(entry.timestamp > 0);
    assert!(entry.refs.iter().any(|r| r == "tag: release"));
}

#[cfg(unix)]
#[test]
fn automatic_history_and_commit_details_never_run_configured_signature_program() {
    use std::{io::Write, os::unix::fs::PermissionsExt, process::Stdio};
    let d = fixture();
    commit(d.path(), "a", "one", "Signed object fixture");
    let raw = Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["cat-file", "commit", "HEAD"])
        .output()
        .unwrap();
    assert!(raw.status.success());
    let divider = raw
        .stdout
        .windows(2)
        .position(|pair| pair == b"\n\n")
        .unwrap();
    let mut signed = raw.stdout[..divider].to_vec();
    signed.extend_from_slice(b"\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n fixture-not-a-real-signature\n -----END PGP SIGNATURE-----\n\n");
    signed.extend_from_slice(&raw.stdout[divider + 2..]);
    let mut hash = Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    hash.stdin.take().unwrap().write_all(&signed).unwrap();
    let hash = hash.wait_with_output().unwrap();
    assert!(
        hash.status.success(),
        "{}",
        String::from_utf8_lossy(&hash.stderr)
    );
    let oid = std::str::from_utf8(&hash.stdout).unwrap().trim();
    git(d.path(), &["update-ref", "refs/heads/main", oid]);
    let marker = d.path().join("signature-program-executed");
    let program = d.path().join(".git/fixture-gpg");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nprintf executed > '{}'\nexit 1\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    git(d.path(), &["config", "gpg.format", "openpgp"]);
    git(
        d.path(),
        &["config", "gpg.program", program.to_str().unwrap()],
    );
    git(d.path(), &["config", "log.showSignature", "true"]);
    // Positive control: the fixture really invokes its own harmless helper when
    // ordinary Git signature rendering is explicitly enabled.
    let control = Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["log", "--show-signature", "--format=%H", "-1"])
        .output()
        .unwrap();
    assert!(control.status.success());
    assert!(
        marker.exists(),
        "fake signature must exercise the configured helper"
    );
    fs::remove_file(&marker).unwrap();
    let repo = Repository::discover(d.path(), &AtomicBool::new(false)).unwrap();
    let page = repo
        .history_reference(0, 10, None, Some("HEAD"), &AtomicBool::new(false))
        .unwrap();
    assert_eq!(page.commits[0].oid, oid);
    assert!(
        !marker.exists(),
        "history must not verify signatures automatically"
    );
    let details = repo
        .commit_details("HEAD", &AtomicBool::new(false))
        .unwrap();
    assert_eq!(details.commit, oid);
    assert!(
        !marker.exists(),
        "commit detail reads must not run signature helpers"
    );
    let page = run(&request(d.path()));
    assert_eq!(page.commits[0].oid, oid);
    assert!(!marker.exists());
}
