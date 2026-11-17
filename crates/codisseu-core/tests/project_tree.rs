//! Real filesystem corpus for lazy Explorer loading, never writes business projects.
use codisseu_core::{AppService, Command, ProjectFileCommand};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn until(app: &mut AppService, condition: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if condition(app) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{} / {:?}",
            app.status,
            app.last_error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn settle(app: &mut AppService) {
    until(app, |a| !a.is_busy());
}
fn open(root: &Path, profile: &Path) -> AppService {
    let mut a = AppService::new(profile.into());
    a.dispatch(Command::OpenWorkspace(root.into())).unwrap();
    settle(&mut a);
    assert!(a.last_error.is_none(), "{:?}", a.last_error);
    a
}
fn load(a: &mut AppService, path: &Path) {
    a.dispatch(Command::LoadDirectory(path.into())).unwrap();
    settle(a);
    assert!(
        a.tree_directory_error(path).is_none(),
        "{:?}",
        a.tree_directory_error(path)
    );
}
fn children(a: &AppService, path: &Path) -> BTreeSet<PathBuf> {
    a.entries
        .iter()
        .filter(|e| e.path.parent() == Some(path))
        .map(|e| e.path.clone())
        .collect()
}
fn disk(path: &Path) -> BTreeSet<PathBuf> {
    fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}
#[test]
fn every_root_sibling_hidden_file_dependency_and_large_branch_remain_accessible() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    fs::create_dir(&root).unwrap();
    for folder in [
        "a-heavy",
        "z-last",
        "vendor",
        "node_modules",
        "target",
        ".git",
        ".idea",
        ".vscode",
    ] {
        fs::create_dir(root.join(folder)).unwrap();
    }
    fs::write(root.join(".env"), "own fixture").unwrap();
    fs::write(root.join("zz-last.php"), "<?php // last").unwrap();
    for n in 0..6001 {
        fs::write(
            root.join("a-heavy").join(format!("File{n:05}.php")),
            "<?php // own",
        )
        .unwrap();
    }
    fs::write(root.join("vendor/autoload.php"), "<?php // vendor fixture").unwrap();
    let mut a = open(&root, &d.path().join("session.json"));
    assert_eq!(children(&a, &root), disk(&root));
    assert_eq!(a.entries.len(), 10);
    assert!(!a.tree_truncated);
    assert!(!a.tree_directory_loaded(&root.join("a-heavy")));
    let status = a.status.clone();
    load(&mut a, &root.join("a-heavy"));
    assert_eq!(
        a.status, status,
        "Directory loading must not leave an in-progress footer"
    );
    assert_eq!(
        children(&a, &root.join("a-heavy")),
        disk(&root.join("a-heavy"))
    );
    assert_eq!(a.entries.len(), 6011);
    assert_eq!(children(&a, &root), disk(&root));
    load(&mut a, &root.join("vendor"));
    a.dispatch(Command::OpenFile(root.join("vendor/autoload.php")))
        .unwrap();
    settle(&mut a);
    assert!(
        a.active_document()
            .unwrap()
            .text()
            .contains("vendor fixture")
    );
    // Refresh reconciles loaded branches instead of reverting to a root-only tree.
    a.dispatch(Command::ProjectFile(ProjectFileCommand::Refresh))
        .unwrap();
    settle(&mut a);
    assert_eq!(children(&a, &root.join("a-heavy")).len(), 6001);
    assert!(a.tree_directory_loaded(&root.join("vendor")));
}
#[test]
fn directory_with_more_than_5000_direct_children_and_depth_over_16_is_complete() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    fs::create_dir(&root).unwrap();
    for n in 0..5001 {
        fs::write(root.join(format!("File{n:05}.php")), "").unwrap();
    }
    let mut deep = root.clone();
    for _ in 0..24 {
        deep.push("nested");
        fs::create_dir(&deep).unwrap();
    }
    fs::write(deep.join("last.php"), "<?php // deep").unwrap();
    let mut a = open(&root, &d.path().join("session.json"));
    assert_eq!(a.entries.len(), 5002);
    assert_eq!(children(&a, &root), disk(&root));
    let mut path = root.clone();
    for _ in 0..24 {
        path.push("nested");
        load(&mut a, &path);
    }
    assert!(
        a.entries
            .iter()
            .any(|e| e.path == deep.join("last.php") && e.depth == 24)
    );
    assert!(!a.tree_truncated);
}
#[test]
fn missing_directory_failure_does_not_hide_siblings_and_refresh_reconciles_it() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    fs::create_dir(&root).unwrap();
    let missing = root.join("missing");
    fs::create_dir(&missing).unwrap();
    fs::write(root.join("last.php"), "last").unwrap();
    let mut a = open(&root, &d.path().join("session.json"));
    fs::remove_dir(&missing).unwrap();
    a.dispatch(Command::LoadDirectory(missing.clone())).unwrap();
    settle(&mut a);
    assert!(
        a.tree_directory_error(&missing).is_some() || !a.entries.iter().any(|e| e.path == missing)
    );
    assert!(a.entries.iter().any(|e| e.path == root.join("last.php")));
    a.dispatch(Command::ProjectFile(ProjectFileCommand::Refresh))
        .unwrap();
    settle(&mut a);
    assert!(a.tree_error().is_none());
    assert_eq!(children(&a, &root), disk(&root));
}
#[test]
fn empty_loaded_directory_watches_changes_and_other_branches_stay_loaded() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    fs::create_dir(&root).unwrap();
    let folder = root.join("empty");
    fs::create_dir(&folder).unwrap();
    let other = root.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("keep.php"), "keep").unwrap();
    let mut a = open(&root, &d.path().join("session.json"));
    load(&mut a, &folder);
    load(&mut a, &other);
    until(&mut a, |a| !a.external_files_pending());
    let path = folder.join("external.php");
    fs::write(&path, "added externally").unwrap();
    until(&mut a, |a| a.entries.iter().any(|e| e.path == path));
    assert!(a.entries.iter().any(|e| e.path == other.join("keep.php")));
    assert!(a.tree_directory_loaded(&folder));
}
#[test]
fn obsolete_workspace_directory_reply_and_duplicate_requests_are_harmless() {
    let d = tempfile::tempdir().unwrap();
    let first = d.path().join("first");
    let second = d.path().join("second");
    fs::create_dir_all(first.join("child")).unwrap();
    fs::create_dir(&second).unwrap();
    fs::write(first.join("child/old.php"), "old").unwrap();
    fs::write(second.join("new.php"), "new").unwrap();
    let mut a = open(&first, &d.path().join("session.json"));
    a.dispatch(Command::LoadDirectory(first.join("child")))
        .unwrap();
    a.dispatch(Command::LoadDirectory(first.join("child")))
        .unwrap();
    a.dispatch(Command::OpenWorkspace(second.clone())).unwrap();
    settle(&mut a);
    assert_eq!(children(&a, &second), disk(&second));
    assert_eq!(a.entries.len(), 1);
    assert!(
        a.dispatch(Command::LoadDirectory(first.join("child")))
            .is_err()
    );
}
#[test]
fn cancellation_discards_the_directory_reply_and_explicit_retry_is_complete() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    fs::create_dir_all(root.join("child")).unwrap();
    fs::write(root.join("child/own.php"), "<?php // own").unwrap();
    let mut a = open(&root, &d.path().join("session.json"));
    a.dispatch(Command::LoadDirectory(root.join("child")))
        .unwrap();
    a.dispatch(Command::CancelAll).unwrap();
    settle(&mut a);
    assert!(!a.tree_directory_loaded(&root.join("child")));
    assert!(children(&a, &root.join("child")).is_empty());
    load(&mut a, &root.join("child"));
    assert_eq!(children(&a, &root.join("child")), disk(&root.join("child")));
}

#[cfg(unix)]
#[test]
fn links_and_cycles_are_visible_but_only_explicit_expansion_follows_them() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let outside = d.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("own.php"), "<?php // own").unwrap();
    symlink(&root, root.join("cycle")).unwrap();
    symlink(&outside, root.join("linked")).unwrap();
    symlink(outside.join("own.php"), root.join("file.php")).unwrap();
    symlink(outside.join("missing"), root.join("broken")).unwrap();
    let mut a = open(&root, &d.path().join("session.json"));
    assert_eq!(a.entries.len(), 4);
    assert_eq!(children(&a, &root), disk(&root));
    load(&mut a, &root.join("cycle"));
    assert_eq!(a.entries.len(), 8);
    assert_eq!(children(&a, &root.join("cycle")).len(), 4);
    load(&mut a, &root.join("linked"));
    assert!(
        a.entries
            .iter()
            .any(|e| e.path == root.join("linked/own.php"))
    );
    assert!(
        a.dispatch(Command::ProjectFile(ProjectFileCommand::CreateFile {
            parent: root.join("linked"),
            name: "must-not-write.php".into()
        }))
        .is_ok()
    );
    settle(&mut a);
    assert!(a.project_files.error.is_some());
    assert!(!outside.join("must-not-write.php").exists());
}
#[test]
#[ignore = "Explicit readonly root requested by user; set CODISSEU_TREE_READONLY_ROOT and output path"]
fn real_project_children_readonly() {
    let root = fs::canonicalize(std::env::var_os("CODISSEU_TREE_READONLY_ROOT").unwrap()).unwrap();
    let d = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let mut a = open(&root, &d.path().join("session.json"));
    let root_ms = started.elapsed().as_millis();
    assert_eq!(children(&a, &root), disk(&root));
    let mut inventory = serde_json::Map::new();
    inventory.insert(
        "root".into(),
        serde_json::json!({"children":children(&a,&root),"load_ms":root_ms}),
    );
    for name in [
        "vendor", "src", "modulos", "html", "includes", ".vscode", ".idea",
    ] {
        let path = root.join(name);
        if !path.is_dir() {
            continue;
        }
        let started = Instant::now();
        load(&mut a, &path);
        assert_eq!(children(&a, &path), disk(&path));
        inventory.insert(name.into(),serde_json::json!({"children_count":children(&a,&path).len(),"load_ms":started.elapsed().as_millis()}));
    }
    assert!(a.documents.is_empty());
    assert!(!a.providers_running());
    assert!(!a.tree_truncated);
    fs::write(
        std::env::var_os("CODISSEU_TREE_READONLY_REPORT").unwrap(),
        serde_json::to_vec_pretty(&inventory).unwrap(),
    )
    .unwrap();
}
