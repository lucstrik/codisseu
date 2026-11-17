use codisseu_git::line_changes::{MAX_BYTES, MAX_LINES, comparison_patch};
use std::sync::atomic::AtomicBool;
#[test]
fn full_context_unicode_normalized_eols_and_empty_versions() {
    let cancel = AtomicBool::new(false);
    let (p, equal) = comparison_patch(
        "one\r\nold ação 🦀\r\nlast\r\n",
        "one\nnew ação 🦀\nlast\n",
        &cancel,
    )
    .unwrap();
    assert!(!equal);
    assert!(p.starts_with("@@"));
    assert!(
        p.contains(" one")
            && p.contains(" last")
            && p.contains("-old ação 🦀")
            && p.contains("+new ação 🦀")
    );
    assert!(!p.contains(".tmp"));
    let (p, equal) = comparison_patch("one\rtwo\r", "one\ntwo\n", &cancel).unwrap();
    assert!(equal && p.contains(" one") && p.contains(" two"));
    assert_eq!(
        comparison_patch("", "", &cancel).unwrap(),
        (String::new(), true)
    );
    assert!(
        comparison_patch("", "new\n", &cancel)
            .unwrap()
            .0
            .contains("+new")
    );
    assert!(
        comparison_patch("old\n", "", &cancel)
            .unwrap()
            .0
            .contains("-old")
    );
}
#[test]
fn bounds_binary_and_cancel_are_errors_instead_of_empty_success() {
    let c = AtomicBool::new(false);
    assert!(comparison_patch("a\0b", "x", &c).is_err());
    assert!(comparison_patch("", "x".repeat(MAX_BYTES + 1).as_str(), &c).is_err());
    assert!(comparison_patch("\r".repeat(MAX_LINES + 1).as_str(), "", &c).is_err());
    assert!(comparison_patch("same", "same", &AtomicBool::new(true)).is_err());
}

fn git(root: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let o = std::process::Command::new("git")
        .args([
            "-c",
            "init.templateDir=",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(o.status.success(), "{o:?}");
    o.stdout
}
#[test]
fn previous_history_is_file_scoped_immutable_and_follows_rename() {
    use codisseu_git::previous::FileHistory;
    use std::fs;
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let c = AtomicBool::new(false);
    git(r, &["init", "-q"]);
    let p = r.join("old.php");
    for n in 1..=3 {
        fs::write(&p, format!("version{n}\n")).unwrap();
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "version"]);
    }
    fs::write(r.join("noise"), "unrelated").unwrap();
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "unrelated"]);
    let h = FileHistory::capture(&p, b"version3\n".to_vec(), &c).unwrap();
    for (step, left, right) in [
        (0, "version2\n", "version3\n"),
        (1, "version2\n", "version3\n"),
        (2, "version1\n", "version2\n"),
        (3, "", "version1\n"),
    ] {
        let pair = h.pair(step, &c).unwrap();
        assert_eq!(pair.before, left.as_bytes());
        assert_eq!(pair.after, right.as_bytes());
        assert_eq!(pair.has_previous, step < 3);
    }
    assert!(h.pair(4, &c).is_err());
    git(r, &["mv", "old.php", "new.php"]);
    git(r, &["commit", "-qm", "rename"]);
    let p = r.join("new.php");
    let h = FileHistory::capture(&p, b"version3\n".to_vec(), &c).unwrap();
    assert_eq!(h.pair(2, &c).unwrap().before, b"version2\n");
    assert_eq!(h.pair(3, &c).unwrap().before, b"version1\n");
    // A branch move after capture does not replace immutable OIDs.
    fs::write(&p, "version4\n").unwrap();
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "new"]);
    assert_eq!(h.pair(2, &c).unwrap().after, b"version3\n");
}
#[test]
fn previous_history_includes_staged_and_unsaved_without_writing() {
    use codisseu_git::previous::FileHistory;
    use std::fs;
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let p = r.join("a.php");
    let c = AtomicBool::new(false);
    git(r, &["init", "-q"]);
    fs::write(&p, "head\n").unwrap();
    git(r, &["add", "."]);
    git(r, &["commit", "-qm", "initial"]);
    fs::write(&p, "staged\n").unwrap();
    git(r, &["add", "."]);
    fs::write(&p, "disk\n").unwrap();
    let h = FileHistory::capture(&p, b"unsaved\n".to_vec(), &c).unwrap();
    for (step, left, right) in [
        (0, "staged\n", "unsaved\n"),
        (1, "head\n", "staged\n"),
        (2, "", "head\n"),
    ] {
        let pair = h.pair(step, &c).unwrap();
        assert_eq!(pair.before, left.as_bytes());
        assert_eq!(pair.after, right.as_bytes());
    }
    assert_eq!(fs::read(&p).unwrap(), b"disk\n");
    assert_eq!(git(r, &["show", ":a.php"]), b"staged\n");
    git(r, &["reset", "--hard", "HEAD"]);
    git(r, &["mv", "a.php", "renamed.php"]);
    let h = FileHistory::capture(&r.join("renamed.php"), b"disk\n".to_vec(), &c).unwrap();
    assert_eq!(h.pair(0, &c).unwrap().before, b"head\n");
    assert_eq!(h.pair(1, &c).unwrap().after, b"head\n");
}
#[test]
fn previous_history_new_unborn_ignored_and_limits() {
    use codisseu_git::previous::FileHistory;
    use std::fs;
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let p = r.join("a.php");
    let c = AtomicBool::new(false);
    git(r, &["init", "-q"]);
    fs::write(&p, "new").unwrap();
    let h = FileHistory::capture(&p, b"new".to_vec(), &c).unwrap();
    let pair = h.pair(0, &c).unwrap();
    assert!(pair.before.is_empty());
    assert_eq!(pair.after, b"new");
    assert!(!pair.has_previous);
    assert!(h.pair(1, &c).is_err());
    fs::write(r.join(".gitignore"), "a.php\n").unwrap();
    assert!(FileHistory::capture(&p, b"new".to_vec(), &c).is_err());
    assert!(FileHistory::capture(&p, vec![b'x'; MAX_BYTES + 1], &c).is_err());
    assert!(FileHistory::capture(&p, b"x".to_vec(), &AtomicBool::new(true)).is_err());
}

#[test]
fn forward_endpoint_is_captured_editor_against_commit_or_index() {
    use codisseu_git::previous::FileHistory;
    use std::fs;
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    let p = r.join("sample.php");
    let c = AtomicBool::new(false);
    git(r, &["init", "-q"]);
    for text in ["one\n", "two\n", "three\n"] {
        fs::write(&p, text).unwrap();
        git(r, &["add", "."]);
        git(r, &["commit", "-qm", "version"]);
    }
    let history = FileHistory::capture(&p, b"three\n".to_vec(), &c).unwrap();
    assert_eq!(history.last_step(), 3);
    let pair = history.working_pair(&c).unwrap();
    assert_eq!(pair.before, b"three\n");
    assert_eq!(pair.after, b"three\n");
    assert!(pair.has_previous);
    assert_eq!(pair.after_label, "Versão do editor");
    fs::write(&p, "staged\n").unwrap();
    git(r, &["add", "."]);
    fs::write(&p, "disk\n").unwrap();
    let history = FileHistory::capture(&p, b"unsaved\n".to_vec(), &c).unwrap();
    let pair = history.working_pair(&c).unwrap();
    assert_eq!(pair.before, b"staged\n");
    assert_eq!(pair.after, b"unsaved\n");
    git(r, &["commit", "-qm", "external head changed"]);
    let pair = history.working_pair(&c).unwrap();
    assert_eq!(pair.before, b"staged\n");
    assert_eq!(pair.after, b"unsaved\n");
    assert!(history.pair(usize::MAX, &c).is_err());
}
