use codisseu_git::line_changes::{self, Change, Kind};
use std::{fs, path::Path, process::Command, sync::atomic::AtomicBool};
fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let o = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o.stdout
}
fn init(path: &Path) {
    git(
        path,
        &["-c", "init.templateDir=", "init", "-q", "-b", "main"],
    );
    git(path, &["config", "core.hooksPath", "/dev/null"]);
    git(path, &["config", "user.name", "Fixture"]);
    git(path, &["config", "user.email", "fixture@example.invalid"]);
    git(path, &["config", "commit.gpgsign", "false"]);
}
#[test]
fn exact_ranges_insert_replace_delete_bof_eof_and_unicode() {
    let c = AtomicBool::new(false);
    for (a, b, expected) in [
        (
            "a\nb\nc\n",
            "a\nnew\nb\nc\n",
            vec![Change {
                lines: 1..2,
                kind: Kind::Added,
            }],
        ),
        (
            "a\nb\nc\n",
            "a\n😀ação\nc\n",
            vec![Change {
                lines: 1..2,
                kind: Kind::Modified,
            }],
        ),
        (
            "a\nb\nc\n",
            "a\nc\n",
            vec![Change {
                lines: 1..1,
                kind: Kind::Deleted,
            }],
        ),
        (
            "a\nb\n",
            "b\n",
            vec![Change {
                lines: 0..0,
                kind: Kind::Deleted,
            }],
        ),
        (
            "a\nb\n",
            "a\n",
            vec![Change {
                lines: 1..1,
                kind: Kind::Deleted,
            }],
        ),
        (
            "a\n",
            "",
            vec![Change {
                lines: 0..0,
                kind: Kind::Deleted,
            }],
        ),
        (
            "",
            "a\nb\n",
            vec![Change {
                lines: 0..2,
                kind: Kind::Added,
            }],
        ),
        (
            "a",
            "a\n",
            vec![Change {
                lines: 0..1,
                kind: Kind::Modified,
            }],
        ),
        ("a\nb\n", "a\r\nb\r\n", vec![]),
    ] {
        assert_eq!(
            line_changes::compare(a, b, &c).unwrap(),
            expected,
            "{a:?} -> {b:?}"
        );
    }
}
#[test]
fn real_index_refresh_stage_unstage_commit_and_paths() {
    let d = tempfile::tempdir().unwrap();
    init(d.path());
    let p = d.path().join("-ação[1]\tfile.php");
    let name = p.file_name().unwrap().to_str().unwrap();
    fs::write(&p, "old\n").unwrap();
    git(d.path(), &["--literal-pathspecs", "add", "--", name]);
    git(d.path(), &["commit", "-qm", "base"]);
    let c = AtomicBool::new(false);
    let index_hash = git(d.path(), &["write-tree"]);
    fs::write(&p, "saved\n").unwrap();
    assert_eq!(
        line_changes::index_baseline(&p, &c).unwrap().unwrap(),
        b"old\n"
    );
    assert_eq!(git(d.path(), &["write-tree"]), index_hash);
    git(d.path(), &["--literal-pathspecs", "add", "--", name]);
    assert!(
        line_changes::compare("saved\n", "saved\n", &c)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        line_changes::index_baseline(&p, &c).unwrap().unwrap(),
        b"saved\n"
    );
    git(
        d.path(),
        &["--literal-pathspecs", "reset", "-q", "HEAD", "--", name],
    );
    assert_eq!(
        line_changes::index_baseline(&p, &c).unwrap().unwrap(),
        b"old\n"
    );
    git(d.path(), &["--literal-pathspecs", "add", "--", name]);
    git(d.path(), &["commit", "-qm", "updated"]);
    assert_eq!(
        line_changes::index_baseline(&p, &c).unwrap().unwrap(),
        b"saved\n"
    );
}
#[test]
fn unborn_untracked_ignored_binary_conflict_and_cancellation() {
    let d = tempfile::tempdir().unwrap();
    init(d.path());
    let c = AtomicBool::new(false);
    let path = d.path().join("new.php");
    fs::write(&path, "new\n").unwrap();
    assert_eq!(
        line_changes::index_baseline(&path, &c).unwrap(),
        Some(vec![])
    );
    fs::write(d.path().join(".gitignore"), "ignored.php\n").unwrap();
    let ignored = d.path().join("ignored.php");
    fs::write(&ignored, "new").unwrap();
    assert_eq!(line_changes::index_baseline(&ignored, &c).unwrap(), None);
    fs::write(&path, b"binary\0value").unwrap();
    git(d.path(), &["add", "new.php"]);
    assert_eq!(line_changes::index_baseline(&path, &c).unwrap(), None);
    assert!(line_changes::compare("x", "y", &AtomicBool::new(true)).is_err());
    assert!(line_changes::compare("x", &"y".repeat(line_changes::MAX_BYTES + 1), &c).is_err());
    fs::write(&path, "base\n").unwrap();
    git(d.path(), &["add", "new.php"]);
    git(d.path(), &["commit", "-qm", "base"]);
    git(d.path(), &["checkout", "-qb", "other"]);
    fs::write(&path, "other\n").unwrap();
    git(d.path(), &["commit", "-qam", "other"]);
    git(d.path(), &["checkout", "-q", "main"]);
    fs::write(&path, "main\n").unwrap();
    git(d.path(), &["commit", "-qam", "main"]);
    let merge = Command::new("git")
        .arg("-C")
        .arg(d.path())
        .args(["merge", "other"])
        .output()
        .unwrap();
    assert!(!merge.status.success());
    assert_eq!(line_changes::index_baseline(&path, &c).unwrap(), None);
}
#[test]
fn nested_repo_and_worktree_resolve_own_index() {
    let d = tempfile::tempdir().unwrap();
    init(d.path());
    let p = d.path().join("file.ts");
    fs::write(&p, "parent\n").unwrap();
    git(d.path(), &["add", "file.ts"]);
    git(d.path(), &["commit", "-qm", "base"]);
    let child = d.path().join("child");
    fs::create_dir(&child).unwrap();
    init(&child);
    let nested = child.join("file.ts");
    fs::write(&nested, "nested\n").unwrap();
    git(&child, &["add", "file.ts"]);
    let c = AtomicBool::new(false);
    assert_eq!(
        line_changes::index_baseline(&nested, &c).unwrap().unwrap(),
        b"nested\n"
    );
    let worktree = d.path().join("wt");
    git(
        d.path(),
        &[
            "worktree",
            "add",
            "-qb",
            "fixture",
            worktree.to_str().unwrap(),
        ],
    );
    assert_eq!(
        line_changes::index_baseline(&worktree.join("file.ts"), &c)
            .unwrap()
            .unwrap(),
        b"parent\n"
    );
}
