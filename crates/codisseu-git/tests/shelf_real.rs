//! Shelf/changelist integration against Git CLI and disposable worktrees only.
use codisseu_git::{
    GitErrorKind, Repository,
    shelf::{self, ShelfAction},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
};

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}
fn text(root: &Path, args: &[&str]) -> String {
    String::from_utf8(git(root, args)).unwrap()
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    repo: Repository,
    cancel: AtomicBool,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        let hooks = root.join(".git/fixture-hooks");
        fs::create_dir(&hooks).unwrap();
        for (k, v) in [
            ("user.name", "Shelf Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("protocol.allow", "never"),
            ("core.hooksPath", hooks.to_str().unwrap()),
        ] {
            git(&root, &["config", k, v]);
        }
        for name in ["a.txt", "b.txt", "delete.txt"] {
            fs::write(root.join(name), format!("base {name}\n")).unwrap();
        }
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "Base"]);
        let cancel = AtomicBool::new(false);
        let repo = Repository::discover(&root, &cancel).unwrap();
        Self {
            _temp: temp,
            root,
            repo,
            cancel,
        }
    }
    fn apply(&self, action: ShelfAction) -> shelf::ShelfReport {
        let p = shelf::prepare_shelf(&self.repo, action, &self.cancel).unwrap();
        let r = shelf::apply_shelf(&self.repo, &p, &self.cancel).unwrap();
        assert!(r.success, "{r:?}");
        r
    }
    fn shelve(&self, paths: &[&str]) -> String {
        self.apply(ShelfAction::Shelve {
            name: "Own shelf 😀".into(),
            paths: paths.iter().map(PathBuf::from).collect(),
        })
        .shelf_id
        .unwrap()
    }
}

#[test]
fn selected_staged_and_working_layers_survive_reopen_and_leave_other_changes_exact() {
    let f = Fixture::new();
    fs::write(f.root.join("a.txt"), "a staged 😀\n").unwrap();
    git(&f.root, &["add", "a.txt"]);
    fs::write(f.root.join("a.txt"), "a working 😀\n").unwrap();
    fs::write(f.root.join("b.txt"), "b staged excluded\n").unwrap();
    git(&f.root, &["add", "b.txt"]);
    fs::write(f.root.join("b.txt"), "b working excluded\n").unwrap();
    let excluded_index = git(&f.root, &["show", ":b.txt"]);
    let excluded_work = fs::read(f.root.join("b.txt")).unwrap();
    let id = f.shelve(&["a.txt"]);
    assert_eq!(git(&f.root, &["show", ":a.txt"]), b"base a.txt\n");
    assert_eq!(fs::read(f.root.join("a.txt")).unwrap(), b"base a.txt\n");
    assert_eq!(git(&f.root, &["show", ":b.txt"]), excluded_index);
    assert_eq!(fs::read(f.root.join("b.txt")).unwrap(), excluded_work);
    assert!(
        git(&f.root, &["stash", "list"]).is_empty(),
        "Shelf must not secretly create a Git stash"
    );
    let reopened = Repository::discover(&f.root, &f.cancel).unwrap();
    let list = shelf::list_shelves(&reopened, &f.cancel).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    let details = shelf::shelf_details(&reopened, &id, &f.cancel).unwrap();
    assert!(!details.staged_patch.is_empty());
    assert!(!details.working_patch.is_empty());
    f.apply(ShelfAction::Unshelve { id: id.clone() });
    assert_eq!(
        git(&f.root, &["show", ":a.txt"]),
        "a staged 😀\n".as_bytes()
    );
    assert_eq!(
        fs::read(f.root.join("a.txt")).unwrap(),
        "a working 😀\n".as_bytes()
    );
    assert_eq!(git(&f.root, &["show", ":b.txt"]), excluded_index);
    assert_eq!(fs::read(f.root.join("b.txt")).unwrap(), excluded_work);
    assert_eq!(
        shelf::list_shelves(&f.repo, &f.cancel).unwrap().len(),
        1,
        "Unshelve retains source until explicit deletion"
    );
    let index = git(&f.root, &["ls-files", "--stage", "-z"]);
    let work = fs::read(f.root.join("a.txt")).unwrap();
    f.apply(ShelfAction::Delete { id });
    assert!(shelf::list_shelves(&f.repo, &f.cancel).unwrap().is_empty());
    assert_eq!(git(&f.root, &["ls-files", "--stage", "-z"]), index);
    assert_eq!(fs::read(f.root.join("a.txt")).unwrap(), work);
}

#[test]
fn unshelve_three_way_merges_nonoverlapping_local_edits_in_both_layers() {
    let f = Fixture::new();
    let base = (0..12).map(|n| format!("line {n}\n")).collect::<String>();
    fs::write(f.root.join("a.txt"), &base).unwrap();
    git(&f.root, &["commit", "-am", "Multiline base"]);
    let shelved = base.replace("line 1\n", "shelved 1\n");
    fs::write(f.root.join("a.txt"), &shelved).unwrap();
    git(&f.root, &["add", "a.txt"]);
    let id = f.shelve(&["a.txt"]);
    let local = base.replace("line 10\n", "local 10\n");
    fs::write(f.root.join("a.txt"), &local).unwrap();
    git(&f.root, &["add", "a.txt"]);
    let p = shelf::prepare_shelf(&f.repo, ShelfAction::Unshelve { id }, &f.cancel).unwrap();
    assert!(p.preview.conflicts.is_empty());
    let r = shelf::apply_shelf(&f.repo, &p, &f.cancel).unwrap();
    assert!(r.success, "{r:?}");
    let merged = base
        .replace("line 1\n", "shelved 1\n")
        .replace("line 10\n", "local 10\n");
    assert_eq!(fs::read_to_string(f.root.join("a.txt")).unwrap(), merged);
    assert_eq!(text(&f.root, &["show", ":a.txt"]), merged);
    assert!(r.recovery_path.unwrap().join("index-original").is_file());
}

#[test]
fn unshelve_overlap_previews_conflict_then_preserves_originals_and_three_index_stages() {
    let f = Fixture::new();
    fs::write(f.root.join("a.txt"), "shelved side\n").unwrap();
    git(&f.root, &["add", "a.txt"]);
    let id = f.shelve(&["a.txt"]);
    fs::write(f.root.join("a.txt"), "current local side\n").unwrap();
    let original_index = fs::read(f.root.join(".git/index")).unwrap();
    let p =
        shelf::prepare_shelf(&f.repo, ShelfAction::Unshelve { id: id.clone() }, &f.cancel).unwrap();
    assert_eq!(p.preview.conflicts, vec![PathBuf::from("a.txt")]);
    assert_eq!(
        fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "current local side\n",
        "preview does not apply anything"
    );
    let r = shelf::apply_shelf(&f.repo, &p, &f.cancel).unwrap();
    assert!(r.success, "{r:?}");
    assert_eq!(r.conflicts, vec![PathBuf::from("a.txt")]);
    assert_eq!(git(&f.root, &["show", ":1:a.txt"]), b"base a.txt\n");
    assert_eq!(git(&f.root, &["show", ":2:a.txt"]), b"current local side\n");
    assert_eq!(git(&f.root, &["show", ":3:a.txt"]), b"shelved side\n");
    let result = fs::read_to_string(f.root.join("a.txt")).unwrap();
    assert!(result.contains("<<<<<<< Current worktree"));
    assert!(result.contains("current local side"));
    assert!(result.contains("shelved side"));
    let recovery = r.recovery_path.unwrap();
    assert_eq!(
        fs::read(recovery.join("0-working")).unwrap(),
        b"current local side\n"
    );
    assert_eq!(
        fs::read(recovery.join("index-original")).unwrap(),
        original_index
    );
    assert_eq!(shelf::list_shelves(&f.repo, &f.cancel).unwrap()[0].id, id);
    let conflict =
        codisseu_git::conflicts::load_conflict(&f.repo, Path::new("a.txt"), &f.cancel).unwrap();
    assert!(
        conflict.file.editable,
        "ordinary conflict editor can resolve Unshelve conflicts"
    );
}

#[test]
fn changelists_are_persistent_grouping_and_active_list_does_not_move_existing_changes() {
    let f = Fixture::new();
    fs::write(f.root.join("a.txt"), "changed a\n").unwrap();
    fs::write(f.root.join("b.txt"), "changed b\n").unwrap();
    git(&f.root, &["add", "a.txt"]);
    let before = fs::read(f.root.join(".git/index")).unwrap();
    let files = [
        fs::read(f.root.join("a.txt")).unwrap(),
        fs::read(f.root.join("b.txt")).unwrap(),
    ];
    f.apply(ShelfAction::CreateChangelist {
        name: "Feature 😀".into(),
        set_default: true,
    });
    let lists = shelf::list_changelists(&f.repo, &f.cancel).unwrap();
    let feature = lists
        .iter()
        .find(|l| l.name == "Feature 😀")
        .unwrap()
        .id
        .clone();
    assert!(lists.iter().find(|l| l.id == feature).unwrap().is_default);
    assert!(
        lists
            .iter()
            .find(|l| l.id == "default")
            .unwrap()
            .paths
            .contains(&PathBuf::from("a.txt")),
        "setting active does not steal existing files"
    );
    fs::write(f.root.join("future.txt"), "untracked future\n").unwrap();
    let lists = shelf::list_changelists(&f.repo, &f.cancel).unwrap();
    assert!(
        lists
            .iter()
            .find(|l| l.id == feature)
            .unwrap()
            .paths
            .contains(&PathBuf::from("future.txt"))
    );
    f.apply(ShelfAction::MoveToChangelist {
        id: feature.clone(),
        paths: vec!["a.txt".into()],
    });
    f.apply(ShelfAction::RenameChangelist {
        id: feature.clone(),
        name: "Renamed".into(),
    });
    let reopened = Repository::discover(&f.root, &f.cancel).unwrap();
    let lists = shelf::list_changelists(&reopened, &f.cancel).unwrap();
    assert!(
        lists
            .iter()
            .find(|l| l.name == "Renamed")
            .unwrap()
            .paths
            .contains(&PathBuf::from("a.txt"))
    );
    assert!(
        shelf::prepare_shelf(
            &f.repo,
            ShelfAction::DeleteChangelist {
                id: feature.clone()
            },
            &f.cancel
        )
        .is_err()
    );
    f.apply(ShelfAction::SetDefaultChangelist {
        id: "default".into(),
    });
    f.apply(ShelfAction::DeleteChangelist { id: feature });
    let lists = shelf::list_changelists(&f.repo, &f.cancel).unwrap();
    assert_eq!(lists.len(), 1);
    assert!(lists[0].paths.contains(&PathBuf::from("a.txt")));
    assert!(lists[0].paths.contains(&PathBuf::from("future.txt")));
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
    assert_eq!(fs::read(f.root.join("a.txt")).unwrap(), files[0]);
    assert_eq!(fs::read(f.root.join("b.txt")).unwrap(), files[1]);
}

#[test]
fn stale_worktree_index_store_or_modified_preview_never_reauthorizes_writes() {
    let f = Fixture::new();
    fs::write(f.root.join("a.txt"), "original draft\n").unwrap();
    let action = || ShelfAction::Shelve {
        name: "Guarded".into(),
        paths: vec!["a.txt".into()],
    };
    let p = shelf::prepare_shelf(&f.repo, action(), &f.cancel).unwrap();
    fs::write(f.root.join("a.txt"), "newer draft\n").unwrap();
    assert_eq!(
        shelf::apply_shelf(&f.repo, &p, &f.cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    assert_eq!(
        fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "newer draft\n"
    );
    let p = shelf::prepare_shelf(&f.repo, action(), &f.cancel).unwrap();
    git(&f.root, &["add", "a.txt"]);
    assert_eq!(
        shelf::apply_shelf(&f.repo, &p, &f.cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    let mut p = shelf::prepare_shelf(&f.repo, action(), &f.cancel).unwrap();
    p.preview.paths.clear();
    assert_eq!(
        shelf::apply_shelf(&f.repo, &p, &f.cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    let p = shelf::prepare_shelf(&f.repo, action(), &f.cancel).unwrap();
    f.apply(ShelfAction::CreateChangelist {
        name: "Concurrent group".into(),
        set_default: false,
    });
    assert_eq!(
        shelf::apply_shelf(&f.repo, &p, &f.cancel).unwrap_err().kind,
        GitErrorKind::Stale
    );
    assert!(shelf::list_shelves(&f.repo, &f.cancel).unwrap().is_empty());
}

#[test]
fn cancellation_and_index_lock_do_not_remove_original_selected_data() {
    let f = Fixture::new();
    fs::write(f.root.join("a.txt"), "precious selected data\n").unwrap();
    let p = shelf::prepare_shelf(
        &f.repo,
        ShelfAction::Shelve {
            name: "Cancelled".into(),
            paths: vec!["a.txt".into()],
        },
        &f.cancel,
    )
    .unwrap();
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        shelf::apply_shelf(&f.repo, &p, &cancelled)
            .unwrap_err()
            .kind,
        GitErrorKind::Cancelled
    );
    fs::write(f.root.join(".git/index.lock"), "owned fixture lock").unwrap();
    let original = fs::read(f.root.join(".git/index")).unwrap();
    assert!(shelf::apply_shelf(&f.repo, &p, &f.cancel).is_err());
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), original);
    assert_eq!(
        fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "precious selected data\n"
    );
    assert!(shelf::list_shelves(&f.repo, &f.cancel).unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(f.root.join(".git/index.lock")).unwrap(),
        "owned fixture lock"
    );
}

#[cfg(unix)]
#[test]
fn real_mid_write_permission_failure_restores_prior_files_and_keeps_durable_shelf() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::create_dir(f.root.join("z-protected")).unwrap();
    fs::write(f.root.join("z-protected/file.txt"), "base protected\n").unwrap();
    git(&f.root, &["add", "."]);
    git(&f.root, &["commit", "-m", "Protected fixture"]);
    fs::write(f.root.join("a.txt"), "selected a\n").unwrap();
    fs::write(f.root.join("z-protected/file.txt"), "selected protected\n").unwrap();
    let p = shelf::prepare_shelf(
        &f.repo,
        ShelfAction::Shelve {
            name: "Recover on actual write failure".into(),
            paths: vec!["a.txt".into(), "z-protected/file.txt".into()],
        },
        &f.cancel,
    )
    .unwrap();
    let original = fs::read(f.root.join(".git/index")).unwrap();
    fs::set_permissions(
        f.root.join("z-protected"),
        fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    let result = shelf::apply_shelf(&f.repo, &p, &f.cancel);
    fs::set_permissions(
        f.root.join("z-protected"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let report = result.unwrap();
    assert!(!report.success);
    assert!(
        !report.outcome_unknown,
        "previous selected file should be restored: {report:?}"
    );
    assert_eq!(
        fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "selected a\n"
    );
    assert_eq!(
        fs::read_to_string(f.root.join("z-protected/file.txt")).unwrap(),
        "selected protected\n"
    );
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), original);
    assert!(
        report
            .recovery_path
            .unwrap()
            .join("metadata.json")
            .is_file()
    );
    let list = shelf::list_shelves(&f.repo, &f.cancel).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        shelf::shelf_details(&f.repo, &list[0].id, &f.cancel)
            .unwrap()
            .info
            .paths
            .len(),
        2
    );
}

#[cfg(unix)]
#[test]
fn rename_delete_add_binary_symlink_executable_and_non_utf8_names_roundtrip() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    };
    let f = Fixture::new();
    let odd = PathBuf::from(std::ffi::OsString::from_vec(b"-odd\xff\nname".to_vec()));
    fs::write(f.root.join(&odd), b"base\0\xff").unwrap();
    symlink("a.txt", f.root.join("link")).unwrap();
    fs::write(f.root.join("tool"), "base tool\n").unwrap();
    git(&f.root, &["add", "."]);
    git(&f.root, &["commit", "-m", "Modes and byte names"]);
    git(&f.root, &["mv", "a.txt", "renamed.txt"]);
    fs::remove_file(f.root.join("delete.txt")).unwrap();
    fs::write(f.root.join("added.txt"), b"new\0binary\xff").unwrap();
    git(&f.root, &["add", "-A"]);
    fs::write(f.root.join(&odd), b"changed\0\xfe").unwrap();
    fs::remove_file(f.root.join("link")).unwrap();
    symlink("renamed.txt", f.root.join("link")).unwrap();
    fs::write(f.root.join("tool"), "changed tool\n").unwrap();
    fs::set_permissions(f.root.join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
    let paths = vec![
        "renamed.txt".into(),
        "delete.txt".into(),
        "added.txt".into(),
        odd.clone(),
        "link".into(),
        "tool".into(),
    ];
    let status = git(&f.root, &["status", "--porcelain=v2", "-z"]);
    let index = git(&f.root, &["ls-files", "--stage", "-z"]);
    let id = f
        .apply(ShelfAction::Shelve {
            name: "Raw modes".into(),
            paths,
        })
        .shelf_id
        .unwrap();
    assert!(git(&f.root, &["status", "--porcelain"]).is_empty());
    f.apply(ShelfAction::Unshelve { id });
    assert_eq!(git(&f.root, &["status", "--porcelain=v2", "-z"]), status);
    assert_eq!(git(&f.root, &["ls-files", "--stage", "-z"]), index);
    assert_eq!(fs::read(f.root.join(&odd)).unwrap(), b"changed\0\xfe");
    assert_eq!(
        fs::read_link(f.root.join("link")).unwrap(),
        Path::new("renamed.txt")
    );
    assert!(
        fs::metadata(f.root.join("tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111
            != 0
    );
    assert_eq!(
        fs::read(f.root.join("added.txt")).unwrap(),
        b"new\0binary\xff"
    );
    assert!(!f.root.join("delete.txt").exists());
}

#[test]
fn oversized_untracked_and_corrupt_archive_are_rejected_without_silent_fallback() {
    let f = Fixture::new();
    fs::write(
        f.root.join("a.txt"),
        vec![b'x'; shelf::MAX_SHELF_FILE_BYTES + 1],
    )
    .unwrap();
    assert!(
        shelf::prepare_shelf(
            &f.repo,
            ShelfAction::Shelve {
                name: "Too large".into(),
                paths: vec!["a.txt".into()]
            },
            &f.cancel
        )
        .is_err()
    );
    fs::write(f.root.join("a.txt"), "small draft\n").unwrap();
    fs::write(f.root.join("untracked"), "retain\n").unwrap();
    assert!(
        shelf::prepare_shelf(
            &f.repo,
            ShelfAction::Shelve {
                name: "Untracked".into(),
                paths: vec!["untracked".into()]
            },
            &f.cancel
        )
        .is_err()
    );
    let id = f.shelve(&["a.txt"]);
    let blob = f
        .repo
        .info()
        .git_dir
        .join("codisseu/shelves")
        .join(&id)
        .join("0-working");
    fs::write(blob, "tampered").unwrap();
    assert!(shelf::shelf_details(&f.repo, &id, &f.cancel).is_err());
    assert!(shelf::prepare_shelf(&f.repo, ShelfAction::Unshelve { id }, &f.cancel).is_err());
    assert_eq!(
        fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "base a.txt\n"
    );
    assert_eq!(
        fs::read_to_string(f.root.join("untracked")).unwrap(),
        "retain\n"
    );
}

#[cfg(unix)]
#[test]
fn storage_is_per_linked_worktree_and_symlink_parents_cannot_escape() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let linked = f._temp.path().join("linked");
    git(
        &f.root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let linked_repo = Repository::discover(&linked, &f.cancel).unwrap();
    fs::write(f.root.join("a.txt"), "main shelf\n").unwrap();
    f.shelve(&["a.txt"]);
    assert!(
        shelf::list_shelves(&linked_repo, &f.cancel)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        shelf::list_changelists(&linked_repo, &f.cancel).unwrap()[0].name,
        "Changes"
    );
    let outside = f._temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), "outside untouched").unwrap();
    symlink(&outside, linked.join("escape")).unwrap();
    assert!(
        shelf::prepare_shelf(
            &linked_repo,
            ShelfAction::Shelve {
                name: "Escape".into(),
                paths: vec!["escape/sentinel".into()]
            },
            &f.cancel
        )
        .is_err()
    );
    symlink(&outside, linked_repo.info().git_dir.join("codisseu")).unwrap();
    assert!(shelf::list_shelves(&linked_repo, &f.cancel).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("sentinel")).unwrap(),
        "outside untouched"
    );
}

#[cfg(unix)]
#[test]
fn shelf_preserves_private_permissions_and_opposite_index_working_changes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::write(
        f.root.join("a.txt"),
        "staged but reversed in working file\n",
    )
    .unwrap();
    git(&f.root, &["add", "a.txt"]);
    fs::write(f.root.join("a.txt"), "base a.txt\n").unwrap();
    fs::set_permissions(f.root.join("a.txt"), fs::Permissions::from_mode(0o600)).unwrap();
    let id = f.shelve(&["a.txt"]);
    assert_eq!(
        fs::metadata(f.root.join("a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    f.apply(ShelfAction::Unshelve { id });
    assert_eq!(
        git(&f.root, &["show", ":a.txt"]),
        b"staged but reversed in working file\n"
    );
    assert_eq!(fs::read(f.root.join("a.txt")).unwrap(), b"base a.txt\n");
    assert_eq!(
        fs::metadata(f.root.join("a.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!git(&f.root, &["diff", "--cached"]).is_empty());
    assert!(!git(&f.root, &["diff"]).is_empty());
}

#[test]
fn local_untracked_collision_and_modify_delete_are_explicit_conflicts_with_backup() {
    for delete in [false, true] {
        let f = Fixture::new();
        let path = if delete { "delete.txt" } else { "new.txt" };
        if delete {
            fs::remove_file(f.root.join(path)).unwrap();
        } else {
            fs::write(f.root.join(path), b"shelved\0binary\xff").unwrap();
        }
        git(&f.root, &["add", "-A"]);
        let id = f.shelve(&[path]);
        let local = b"precious local\0binary\xfe";
        fs::write(f.root.join(path), local).unwrap();
        let prepared =
            shelf::prepare_shelf(&f.repo, ShelfAction::Unshelve { id: id.clone() }, &f.cancel)
                .unwrap();
        assert_eq!(prepared.preview.conflicts, vec![PathBuf::from(path)]);
        let report = shelf::apply_shelf(&f.repo, &prepared, &f.cancel).unwrap();
        assert!(report.success, "{report:?}");
        assert_eq!(fs::read(f.root.join(path)).unwrap(), local);
        assert_eq!(git(&f.root, &["show", &format!(":2:{path}")]), local);
        let stages = text(&f.root, &["ls-files", "--unmerged"]);
        if delete {
            assert!(stages.contains(" 1\tdelete.txt"));
            assert!(!stages.contains(" 3\tdelete.txt"));
        } else {
            assert!(!stages.contains(" 1\tnew.txt"));
            assert_eq!(
                git(&f.root, &["show", ":3:new.txt"]),
                b"shelved\0binary\xff"
            );
        }
        assert_eq!(
            fs::read(report.recovery_path.unwrap().join("0-working")).unwrap(),
            local
        );
        assert_eq!(shelf::list_shelves(&f.repo, &f.cancel).unwrap()[0].id, id);
    }
}

#[test]
fn full_shelf_message_round_trips_while_changelist_names_stay_single_line() {
    let f = Fixture::new();
    let message = "Corrigir integração 😀\n\nCorpo completo com acentos.\r\n\tPróximo passo\n";
    fs::write(f.root.join("a.txt"), "selected message fixture\n").unwrap();
    let report = f.apply(ShelfAction::Shelve {
        name: message.into(),
        paths: vec!["a.txt".into()],
    });
    let id = report.shelf_id.unwrap();
    let reopened = Repository::discover(&f.root, &f.cancel).unwrap();
    let list = shelf::list_shelves(&reopened, &f.cancel).unwrap();
    assert_eq!(list[0].name, message);
    assert_eq!(
        shelf::shelf_details(&reopened, &id, &f.cancel)
            .unwrap()
            .info
            .name,
        message
    );
    f.apply(ShelfAction::Unshelve { id });
    assert_eq!(
        fs::read_to_string(f.root.join("a.txt")).unwrap(),
        "selected message fixture\n"
    );

    for invalid in [
        "\n\t ".to_owned(),
        "bad\0message".into(),
        "bad\u{1b}message".into(),
        "x".repeat(65537),
    ] {
        let error = shelf::prepare_shelf(
            &f.repo,
            ShelfAction::Shelve {
                name: invalid,
                paths: vec!["a.txt".into()],
            },
            &f.cancel,
        )
        .unwrap_err();
        assert!(error.message.contains("Mensagem de Shelf"), "{error}");
    }
    for action in [
        ShelfAction::CreateChangelist {
            name: "invalid\nlist".into(),
            set_default: false,
        },
        ShelfAction::RenameChangelist {
            id: "default".into(),
            name: "invalid\nrename".into(),
        },
    ] {
        let error = shelf::prepare_shelf(&f.repo, action, &f.cancel).unwrap_err();
        assert!(error.message.contains("Nome"), "{error}");
    }
    assert_eq!(
        shelf::list_changelists(&f.repo, &f.cancel).unwrap()[0].name,
        "Changes"
    );
}
