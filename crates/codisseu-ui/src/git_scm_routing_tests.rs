//! Cached rows from inactive repositories must never mutate the selected neighbour.
use super::*;

#[test]
fn cached_inactive_file_rebinds_before_prepare_and_stale_rows_are_refused() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("second");
    fs::create_dir(&second).unwrap();
    cli(&second, &["init", "-b", "second-main"]);
    cli(&second, &["config", "user.name", "Second Fixture"]);
    cli(&second, &["config", "user.email", "second@example.invalid"]);
    cli(&second, &["config", "commit.gpgsign", "false"]);
    cli(&second, &["config", "core.hooksPath", "/dev/null"]);
    fs::write(second.join("base.txt"), "original\n").unwrap();
    cli(&second, &["add", "base.txt"]);
    cli(&second, &["commit", "-m", "Second base"]);
    fs::write(second.join("second-only.txt"), "new version\n").unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut h);
    h.state_mut()
        .app
        .service
        .dispatch(Command::SelectGitRepository(first.clone()))
        .unwrap();
    settle(&mut h);
    h.set_size(egui::vec2(1280., 1400.));
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    assert_eq!(h.state().app.service.git.repositories.len(), 2);
    assert!(
        h.state()
            .app
            .service
            .git
            .repositories
            .iter()
            .all(|repo| repo.entries.is_some())
    );
    let original_index = cli(&first, &["write-tree"]);
    h.get_by_label(&format!("Stage second-only.txt in {}", second.display()))
        .click();
    settle(&mut h);
    assert_eq!(
        h.state().app.service.git.repo.as_ref().unwrap().root,
        second
    );
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(cli(&second, &["diff", "--cached", "--name-only"]).is_empty());
    h.get_by_label("Stage second-only.txt").click();
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    assert!(!h.state().app.git_ui.diff_open, "diff unexpectedly open");
    h.get_by_label("Apply").click();
    h.step();
    settle(&mut h);

    assert_eq!(cli(&second, &["show", ":second-only.txt"]), "new version\n");
    assert_eq!(cli(&first, &["write-tree"]), original_index);

    fs::write(second.join("stale-only.txt"), "will disappear\n").unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::SelectGitRepository(first.clone()))
        .unwrap();
    settle(&mut h);
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    assert!(
        h.query_by_label(&format!("Stage stale-only.txt in {}", second.display()))
            .is_some()
    );
    fs::remove_file(second.join("stale-only.txt")).unwrap();
    let second_index = cli(&second, &["write-tree"]);
    h.get_by_label(&format!("Stage stale-only.txt in {}", second.display()))
        .click();
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert!(h.state().app.git_ui.notice.contains("arquivo mudou"));
    assert_eq!(cli(&second, &["write-tree"]), second_index);
    assert_eq!(cli(&first, &["write-tree"]), original_index);
}

#[test]
fn explorer_nested_repository_stage_rebinds_and_root_history_does_not_use_empty_path() {
    let (_dir, parent, mut h) = fixture();
    let nested = parent.join("child-repository");
    fs::create_dir(&nested).unwrap();
    cli(&nested, &["init", "-b", "child"]);
    cli(&nested, &["config", "user.name", "Nested Fixture"]);
    cli(&nested, &["config", "user.email", "nested@example.invalid"]);
    cli(&nested, &["config", "commit.gpgsign", "false"]);
    cli(&nested, &["config", "core.hooksPath", "/dev/null"]);
    fs::write(nested.join("item.txt"), "original").unwrap();
    cli(&nested, &["add", "item.txt"]);
    cli(&nested, &["commit", "-m", "Nested base"]);
    fs::write(nested.join("item.txt"), "changed").unwrap();
    h.state_mut().app.git_discover_repositories();
    settle(&mut h);
    assert!(
        h.state()
            .app
            .service
            .git
            .repositories
            .iter()
            .any(|repo| repo.root == nested)
    );
    let parent_index = cli(&parent, &["write-tree"]);
    h.state_mut()
        .app
        .git_explorer_target("stage", nested.join("item.txt"));
    settle(&mut h);
    assert_eq!(
        h.state().app.service.git.repo.as_ref().unwrap().root,
        nested
    );
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(cli(&nested, &["show", ":item.txt"]), "changed");
    assert_eq!(cli(&parent, &["write-tree"]), parent_index);
    h.state_mut().app.git_explorer_target("history", nested);
    settle(&mut h);
    assert!(h.state().app.show_activity);
    assert!(matches!(h.state().app.tool_tab, ToolTab::Git));
    assert!(h.state().app.service.git.error.is_none());
}

#[test]
fn stage_click_waits_for_accepted_save_ack_instead_of_discarding_the_intent() {
    let (_dir, root, mut h) = fixture();
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    fs::write(
        root.join("AGENTS.md"),
        "# Own fixture\n\nOriginal instructions.\n",
    )
    .unwrap();
    cli(&root, &["add", "AGENTS.md"]);
    cli(&root, &["commit", "-m", "Own fixture instructions"]);
    fs::write(
        root.join("AGENTS.md"),
        "# Own fixture\n\nExternal change.\n",
    )
    .unwrap();
    h.state_mut()
        .app
        .send(Command::OpenFile(root.join("AGENTS.md")));
    settle(&mut h);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    let old_head = cli(&root, &["rev-parse", "HEAD"]);
    let old_index = cli(&root, &["write-tree"]);
    let doc = h.state().app.service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    let saved = "# Own fixture\n\nSaved before staging ação 😀.\n";
    h.state_mut().app.send(Command::Edit {
        id,
        expected_version: version,
        text: saved.into(),
    });
    h.state_mut().app.send(Command::Save { id });
    assert!(h.state().app.service.document_save_pending(id));
    // Keep the accepted Save ACK undelivered while the actual + widget is clicked.
    // The write may already have reached disk; neither case permits losing Stage.
    h.state_mut().pause_delivery = true;
    h.get_by_label("Stage AGENTS.md").click();
    h.step();
    h.state_mut().app.poll_selected_git_ui(); // Trust, if needed.
    h.state_mut().app.poll_selected_git_ui(); // Prepare must remain queued for Save.
    assert!(
        h.state().app.git_ui.preview_requested,
        "Stage intent was lost: {} {:?}",
        h.state().app.git_ui.notice,
        h.state().app.service.git.error
    );
    assert_eq!(cli(&root, &["write-tree"]), old_index);
    h.state_mut().pause_delivery = false;
    settle(&mut h);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert_eq!(cli(&root, &["show", ":AGENTS.md"]), saved);
    assert_eq!(fs::read_to_string(root.join("AGENTS.md")).unwrap(), saved);
    assert_eq!(cli(&root, &["rev-parse", "HEAD"]), old_head);
    assert!(h.state().app.service.git.error.is_none());
}

#[test]
fn stage_in_other_repository_finishes_while_first_save_ack_is_undelivered() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("second");
    fs::create_dir(&second).unwrap();
    cli(&second, &["init", "-b", "main"]);
    cli(&second, &["config", "user.name", "Other Stage Fixture"]);
    cli(&second, &["config", "user.email", "stage@example.invalid"]);
    cli(&second, &["config", "commit.gpgsign", "false"]);
    cli(&second, &["config", "core.hooksPath", "/dev/null"]);
    fs::write(second.join("other.txt"), "original\n").unwrap();
    cli(&second, &["add", "other.txt"]);
    cli(&second, &["commit", "-m", "Other stage base"]);
    fs::write(second.join("other.txt"), "second change ação 😀\n").unwrap();
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut h);
    assert_eq!(h.state().app.service.workspace_path(), Some(dir.path()));
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    // Warm both bindings before retaining just the first repository's file ACK.
    h.state_mut()
        .app
        .send(Command::SelectGitRepository(second.clone()));
    settle(&mut h);
    h.state_mut()
        .app
        .send(Command::SelectGitRepository(first.clone()));
    settle(&mut h);
    h.state_mut()
        .app
        .send(Command::OpenFile(first.join("app.js")));
    settle(&mut h);
    let first_index = cli(&first, &["write-tree"]);
    let first_head = cli(&first, &["rev-parse", "HEAD"]);
    let second_head = cli(&second, &["rev-parse", "HEAD"]);
    let doc = h.state().app.service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut().app.send(Command::Edit {
        id,
        expected_version,
        text: "first accepted save\n".into(),
    });
    h.state_mut().app.send(Command::Save { id });
    h.state_mut().pause_delivery = true;
    h.get_by_label(&format!("Stage other.txt in {}", second.display()))
        .click();
    h.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while cli(&second, &["diff", "--cached", "--name-only"]).trim() != "other.txt" {
        // Advance only Git. AppService::poll would also deliver the Save ACK.
        h.state_mut().app.service.git.poll();
        h.state_mut().app.poll_selected_git_ui();
        h.step();
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            h.state().app.git_ui.notice,
            h.state().app.service.git.error
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(h.state().app.service.document_save_pending(id));
    assert_eq!(
        cli(&second, &["show", ":other.txt"]),
        "second change ação 😀\n"
    );
    assert_eq!(cli(&first, &["write-tree"]), first_index);
    assert_eq!(cli(&first, &["rev-parse", "HEAD"]), first_head);
    assert_eq!(cli(&second, &["rev-parse", "HEAD"]), second_head);
    assert_eq!(
        fs::read_to_string(second.join("other.txt")).unwrap(),
        "second change ação 😀\n"
    );
    h.state_mut().pause_delivery = false;
    settle(&mut h);
}

#[cfg(unix)]
#[test]
fn queued_stage_uses_disk_after_save_failure_and_preserves_dirty_buffer_and_error() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, root, mut h) = fixture();
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    fs::write(root.join("helper.ts"), "export const unchanged = true;\n").unwrap();
    cli(&root, &["add", "helper.ts"]);
    cli(
        &root,
        &["commit", "-m", "Clean helper for post-stage reload"],
    );
    h.state_mut()
        .app
        .send(Command::OpenFile(root.join("helper.ts")));
    settle(&mut h);
    fs::write(root.join("app.js"), "external disk change\n").unwrap();
    h.state_mut()
        .app
        .send(Command::OpenFile(root.join("app.js")));
    settle(&mut h);
    h.state_mut().app.git_send(GitCommand::Refresh);
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    let old_index = cli(&root, &["write-tree"]);
    let old_head = cli(&root, &["rev-parse", "HEAD"]);
    let doc = h.state().app.service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut().app.send(Command::Edit {
        id,
        expected_version,
        text: "local unsaved ação 😀\n".into(),
    });
    let path = root.join("app.js");
    let permissions = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    h.state_mut().app.send(Command::Save { id });
    assert!(h.state().app.service.document_save_pending(id));
    h.state_mut().pause_delivery = true;
    h.get_by_label("Stage app.js").click();
    h.step();
    h.state_mut().app.poll_selected_git_ui();
    h.state_mut().app.poll_selected_git_ui();
    assert!(h.state().app.git_ui.preview_requested);
    h.state_mut().pause_delivery = false;
    settle(&mut h);
    assert_ne!(cli(&root, &["write-tree"]), old_index);
    assert_eq!(cli(&root, &["show", ":app.js"]), "external disk change\n");
    assert_eq!(cli(&root, &["rev-parse", "HEAD"]), old_head);
    assert_eq!(fs::read_to_string(&path).unwrap(), "external disk change\n");
    let doc = h.state().app.service.active_document().unwrap();
    assert_eq!(doc.text(), "local unsaved ação 😀\n");
    assert!(doc.is_dirty());
    assert!(!h.state().app.git_ui.preview_requested);
    let error = h
        .state()
        .app
        .service
        .last_error
        .clone()
        .expect("Save error must survive index-only Stage");
    assert!(error.contains("somente leitura"), "{error}");
    let ctx = h.ctx.clone();
    h.state().app.collect_status_messages(&ctx);
    assert_eq!(crate::status_messages::current(&ctx), Some(error));
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn stage_and_unstage_use_disk_with_auto_save_off_and_keep_the_editor_dirty() {
    let (_dir, root, mut h) = fixture();
    h.state_mut().app.send(Command::SetAutoSave {
        enabled: false,
        delay_ms: 1000,
    });
    h.state_mut().app.send(Command::SetGitConfirmActions(false));
    settle(&mut h);
    fs::write(root.join("app.js"), "disk version ação 😀\n").unwrap();
    h.state_mut()
        .app
        .send(Command::OpenFile(root.join("app.js")));
    settle(&mut h);
    let doc = h.state().app.service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut().app.send(Command::Edit {
        id,
        expected_version,
        text: "UNSAVED BUFFER ação 😀\n".into(),
    });
    h.state_mut().app.git_send(GitCommand::Refresh);
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    let version = h.state().app.service.active_document().unwrap().version;
    let head = cli(&root, &["rev-parse", "HEAD"]);
    let index = cli(&root, &["write-tree"]);
    h.get_by_label("Stage app.js").click();
    settle(&mut h);
    assert_eq!(cli(&root, &["show", ":app.js"]), "disk version ação 😀\n");
    for unstage in [false, true] {
        if unstage {
            h.get_by_label("Unstage app.js").click();
            settle(&mut h);
        }
        let doc = h.state().app.service.active_document().unwrap();
        assert_eq!(doc.text(), "UNSAVED BUFFER ação 😀\n");
        assert!(doc.is_dirty());
        assert_eq!(doc.version, version);
        assert_eq!(
            fs::read_to_string(root.join("app.js")).unwrap(),
            "disk version ação 😀\n"
        );
        assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
        assert!(h.state().app.git_ui.dialog.is_none());
    }
    assert_eq!(cli(&root, &["write-tree"]), index);
    assert!(!h.state().app.service.editor_preferences().auto_save);
}
