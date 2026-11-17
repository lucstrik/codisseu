//! Modal flows keep the actual private Shelf preview and staged/working layers.
use super::*;

fn shelve_fixture() -> (tempfile::TempDir, PathBuf, GitHarness, String) {
    let (dir, root, mut h) = fixture();
    fs::write(root.join("app.js"), "selected staged layer\n").unwrap();
    cli(&root, &["add", "app.js"]);
    fs::write(root.join("app.js"), "selected working layer\n").unwrap();
    h.state_mut().app.git_ui.message = "Commit draft must remain".into();
    h.state_mut()
        .app
        .git_prepare_shelf(codisseu_core::git::shelf::ShelfAction::Shelve {
            name: "Greeting review\nFull comment 😀".into(),
            paths: vec!["app.js".into()],
        });
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    let id = h.state().app.service.git.shelves[0].id.clone();
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "-- original 😀\n"
    );
    (dir, root, h, id)
}

#[test]
fn unshelve_cancel_is_read_only_and_preview_cancel_restores_comment() {
    let (_dir, root, mut h, id) = shelve_fixture();
    let before = cli(&root, &["status", "--porcelain=v1"]);
    h.state_mut().app.git_ui.dialog = Some(crate::git_shelf::unshelve_dialog(id.clone()));
    settle(&mut h);
    assert!(
        !h.get_by_label("Unshelve Changes")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert_eq!(cli(&root, &["status", "--porcelain=v1"]), before);
    h.state_mut().app.git_ui.dialog = Some(crate::git_shelf::unshelve_dialog(id.clone()));
    settle(&mut h);
    h.get_by_label("Unshelve Changes").click();
    settle(&mut h);
    assert!(h.state().app.service.git.preview.is_some());
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(
        matches!(h.state().app.git_ui.dialog.as_ref(), Some(GitDialog::Unshelve { id: actual, .. }) if actual == &id)
    );
    assert_eq!(
        h.state()
            .app
            .service
            .git
            .shelf_details
            .as_ref()
            .unwrap()
            .info
            .name,
        "Greeting review\nFull comment 😀"
    );
    assert_eq!(cli(&root, &["status", "--porcelain=v1"]), before);
    assert_eq!(h.state().app.git_ui.message, "Commit draft must remain");
}

#[test]
fn unshelve_modal_submits_real_preview_and_retains_shelf() {
    let (_dir, root, mut h, id) = shelve_fixture();
    h.state_mut().app.git_ui.dialog = Some(crate::git_shelf::unshelve_dialog(id));
    settle(&mut h);
    h.get_by_label("Unshelve Changes").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "-- original 😀\n",
        "Opening preview must not apply"
    );
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "selected working layer\n"
    );
    assert_eq!(cli(&root, &["show", ":app.js"]), "selected staged layer\n");
    assert_eq!(h.state().app.service.git.shelves.len(), 1);
    assert_eq!(h.state().app.git_ui.message, "Commit draft must remain");
    assert!(!h.state().app.git_ui.applying);
}

#[test]
fn shelve_readonly_diff_switches_real_index_layer_without_writing() {
    let (_dir, root, mut h) = fixture();
    fs::write(root.join("app.js"), "staged-only marker\n").unwrap();
    cli(&root, &["add", "app.js"]);
    fs::write(root.join("app.js"), "working-only marker\n").unwrap();
    let before = cli(&root, &["status", "--porcelain=v1"]);
    h.state_mut().app.git_open_shelve(vec!["app.js".into()]);
    settle(&mut h);
    let diff = h.state().app.service.git.diff.as_ref().unwrap();
    assert_eq!(diff.side, DiffSide::Working);
    assert!(String::from_utf8_lossy(&diff.bytes).contains("+working-only marker"));
    eprintln!(
        "Index rect {:?}, Cancel {:?}",
        h.get_by_label("Index").rect(),
        h.get_by_label("Cancel").rect()
    );
    h.get_by_label("Index").click();
    settle(&mut h);
    eprintln!(
        "status {} error {:?} queue {:?}",
        h.state().app.service.git.status,
        h.state().app.service.git.error,
        h.state().app.git_ui.queue.len()
    );
    let diff = h.state().app.service.git.diff.as_ref().unwrap();
    assert_eq!(diff.side, DiffSide::Index);
    let patch = String::from_utf8_lossy(&diff.bytes);
    assert!(patch.contains("+staged-only marker"));
    assert!(!patch.contains("working-only marker"));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["status", "--porcelain=v1"]), before);
    assert_eq!(cli(&root, &["show", ":app.js"]), "staged-only marker\n");
    assert!(h.state().app.service.git.shelves.is_empty());
}

#[test]
fn unshelve_options_restore_only_selected_file_and_keep_draft_on_cancel() {
    let (_dir, root, mut h) = fixture();
    fs::write(root.join("other.php"), "original\n").unwrap();
    cli(&root, &["add", "other.php"]);
    cli(&root, &["commit", "-m", "Second fixture file"]);
    fs::write(root.join("app.js"), "restored JS\n").unwrap();
    fs::write(root.join("other.php"), "retained PHP\n").unwrap();
    h.state_mut()
        .app
        .git_prepare_shelf(codisseu_core::git::shelf::ShelfAction::Shelve {
            name: "Partial shelf".into(),
            paths: vec!["app.js".into(), "other.php".into()],
        });
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    let id = h.state().app.service.git.shelves[0].id.clone();
    h.state_mut().app.git_ui.dialog = Some(crate::git_shelf::unshelve_dialog(id.clone()));
    settle(&mut h);
    h.get_by_label("other.php").click();
    // The field starts with the shelf title; use the retained form state to set a
    // deterministic target while exercising selection/check controls via widgets.
    if let Some(GitDialog::Unshelve { name, comment, .. }) = &mut h.state_mut().app.git_ui.dialog {
        *name = "Review target".into();
        *comment = "Only JavaScript".into();
    }
    h.get_by_label("Set active").click();
    h.get_by_label("Remove successfully applied files from the shelf")
        .click();
    h.get_by_label("Unshelve Changes").click();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(
        matches!(&h.state().app.git_ui.dialog, Some(GitDialog::Unshelve { selected, name, comment, active: true, remove_applied: true, .. }) if selected.len()==1 && selected.contains(Path::new("app.js")) && name=="Review target" && comment=="Only JavaScript")
    );
    assert_eq!(
        fs::read_to_string(root.join("other.php")).unwrap(),
        "original\n"
    );
    h.get_by_label("Unshelve Changes").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "restored JS\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("other.php")).unwrap(),
        "original\n"
    );
    let shelf = h
        .state()
        .app
        .service
        .git
        .shelves
        .iter()
        .find(|s| s.id == id)
        .unwrap();
    assert_eq!(shelf.files.len(), 1);
    assert_eq!(shelf.files[0].path, Path::new("other.php"));
    let list = h
        .state()
        .app
        .service
        .git
        .changelists
        .iter()
        .find(|l| l.name == "Review target")
        .unwrap();
    assert!(list.is_default);
    assert_eq!(list.comment, "Only JavaScript");
    assert!(list.paths.iter().any(|p| p == Path::new("app.js")));
}

#[test]
fn deleted_only_shelf_remains_discoverable_and_restore_requires_review() {
    let (_dir, root, mut h, id) = shelve_fixture();
    h.state_mut()
        .app
        .git_prepare_shelf(codisseu_core::git::shelf::ShelfAction::Delete { id: id.clone() });
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert!(h.state().app.service.git.shelves.is_empty());
    assert_eq!(h.state().app.service.git.deleted_shelves.len(), 1);
    h.state_mut().app.git_ui.commit_open = true;
    h.state_mut().app.git_ui.commit_tab = 0;
    settle(&mut h);
    h.get_by_label("Shelf").click();
    settle(&mut h);
    h.get_by_label("Recently Deleted").click();
    settle(&mut h);
    let row = "Deleted shelf Greeting review\nFull comment 😀";
    h.get_by_label(row).click_secondary();
    h.run();
    h.get_by_label("Restore").click();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(h.state().app.service.git.shelves.is_empty());
    h.get_by_label(row).click_secondary();
    h.run();
    h.get_by_label("Restore").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(h.state().app.service.git.shelves[0].id, id);
    assert!(h.state().app.service.git.deleted_shelves.is_empty());
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "-- original 😀\n"
    );
    assert_eq!(h.state().app.git_ui.message, "Commit draft must remain");
}
