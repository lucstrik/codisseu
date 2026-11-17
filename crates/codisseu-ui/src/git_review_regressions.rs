//! Review regressions: real Git and native widgets, with independent index checks.
use super::*;
use codisseu_core::git::{rebase::RebaseAction, shelf::ShelfAction};

#[test]
fn hunk_preview_cancel_returns_to_diff_and_apply_stages_only_selected_hunk() {
    let (_dir, root, mut h) = fixture();
    let base = (0..50).map(|n| format!("line {n}\n")).collect::<String>();
    fs::write(root.join("app.js"), &base).unwrap();
    cli(&root, &["commit", "-am", "Multihunk base"]);
    let working = base
        .replace("line 5\n", "selected change 5\n")
        .replace("line 40\n", "excluded change 40\n");
    fs::write(root.join("app.js"), &working).unwrap();
    h.state_mut()
        .app
        .git_open_diff("app.js".into(), DiffSide::Working);
    settle(&mut h);
    h.get_by_label("Hunk 1").click();
    h.run();
    h.get_by_label("Stage Selected Hunks").click();
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(h.state().app.git_ui.diff_open);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert_eq!(cli(&root, &["show", ":app.js"]), base);
    assert_eq!(fs::read_to_string(root.join("app.js")).unwrap(), working);
    // The actual diff and its hunk controls must be visible again, not a hidden modal.
    h.get_by_label("Stage Selected Hunks").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        cli(&root, &["show", ":app.js"]),
        base.replace("line 5\n", "selected change 5\n")
    );
    assert_eq!(fs::read_to_string(root.join("app.js")).unwrap(), working);
    assert!(!h.state().app.git_ui.applying);
    assert_eq!(
        h.get_by_label("Hunk 1").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False),
        "after staging, the previously excluded hunk must not inherit ordinal selection"
    );
    assert!(
        h.get_by_label("Stage Selected Hunks")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Close").click();
    h.run();
    assert!(!h.state().app.git_ui.diff_open);
}

#[test]
fn external_change_before_hunk_prepare_cannot_retarget_selected_ordinal() {
    let (_dir, root, mut h) = fixture();
    let base = (0..80).map(|n| format!("line {n}\n")).collect::<String>();
    fs::write(root.join("app.js"), &base).unwrap();
    cli(&root, &["commit", "-am", "Hunk identity base"]);
    let displayed = base
        .replace("line 30\n", "previously selected 30\n")
        .replace("line 65\n", "excluded 65\n");
    fs::write(root.join("app.js"), &displayed).unwrap();
    h.state_mut()
        .app
        .git_open_diff("app.js".into(), DiffSide::Working);
    settle(&mut h);
    h.get_by_label("Hunk 1").click();
    h.run();
    let newer = displayed.replace("line 2\n", "new external change 2\n");
    fs::write(root.join("app.js"), &newer).unwrap();
    h.get_by_label("Stage Selected Hunks").click();
    settle(&mut h);
    if matches!(h.state().app.git_ui.dialog, Some(GitDialog::Preview)) {
        h.get_by_label("Apply").click();
        settle(&mut h);
    }
    assert_eq!(
        cli(&root, &["show", ":app.js"]),
        base,
        "selection came from the old diff; an inserted earlier hunk must not silently replace it"
    );
    assert_eq!(fs::read_to_string(root.join("app.js")).unwrap(), newer);
    assert!(
        h.state().app.service.git.error.is_some() || !h.state().app.git_ui.notice.is_empty(),
        "a stale visible hunk selection must require a fresh review"
    );
}

#[test]
fn first_rebase_review_and_dirty_prepare_keep_the_edited_plan_and_buffer() {
    let (_dir, root, mut h) = fixture();
    let base = cli(&root, &["rev-parse", "HEAD"]).trim().to_owned();
    for subject in ["Alpha", "Bravo"] {
        fs::write(root.join(format!("{subject}.txt")), subject).unwrap();
        cli(&root, &["add", "."]);
        cli(&root, &["commit", "-m", subject]);
    }
    let original_head = cli(&root, &["rev-parse", "HEAD"]);
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenFile(root.join("app.js")))
        .unwrap();
    settle(&mut h);
    let doc = h.state().app.service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    h.state_mut()
        .app
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "unsaved editor text 😀\n".into(),
        })
        .unwrap();
    assert!(
        !h.state().app.service.git.trusted,
        "this must cover the first trusted transition"
    );
    h.state_mut().app.git_open_interactive_rebase(base);
    settle(&mut h);
    assert!(h.state().app.service.git.trusted);
    h.get_by_label("Move commit down").click();
    h.run();
    h.get_by_label("Reword").click();
    h.run();
    replace_input(
        &mut h,
        "Rebase commit message",
        "Keep the reordered reword draft\n\nFull body 😀",
    );
    h.get_by_label("Done").click();
    h.run();
    let desired = match &h.state().app.git_ui.dialog {
        Some(GitDialog::InteractiveRebase { steps, .. }) => {
            assert_eq!(steps.len(), 2);
            assert_eq!(steps[1].action, RebaseAction::Reword);
            steps
                .iter()
                .map(|s| (s.oid.clone(), s.action, s.message.clone()))
                .collect::<Vec<_>>()
        }
        _ => panic!("interactive review missing"),
    };
    h.get_by_label("Start Rebasing").click();
    settle(&mut h);
    assert!(
        h.state().app.git_ui.notice.contains("Salve"),
        "{}",
        h.state().app.git_ui.notice
    );
    match &h.state().app.git_ui.dialog {
        Some(GitDialog::InteractiveRebase { steps, .. }) => {
            assert_eq!(
                steps
                    .iter()
                    .map(|s| (s.oid.clone(), s.action, s.message.clone()))
                    .collect::<Vec<_>>(),
                desired
            );
        }
        _ => panic!("failed Prepare discarded the edited rebase plan"),
    }
    assert_eq!(cli(&root, &["rev-parse", "HEAD"]), original_head);
    assert_eq!(
        h.state().app.service.active_document().unwrap().text(),
        "unsaved editor text 😀\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "-- original 😀\n"
    );
}

#[test]
fn changelist_names_staged_or_conflicts_are_not_staging_or_conflict_roles() {
    for list_name in ["Staged", "Conflicts"] {
        let (_dir, root, mut h) = fixture();
        fs::write(root.join("app.js"), "real staged layer\n").unwrap();
        cli(&root, &["add", "app.js"]);
        fs::write(root.join("app.js"), "real working layer\n").unwrap();
        h.state_mut()
            .app
            .git_prepare_shelf(ShelfAction::CreateChangelist {
                name: list_name.into(),
                set_default: false,
            });
        settle(&mut h);
        h.get_by_label("Apply").click();
        settle(&mut h);
        let id = h
            .state()
            .app
            .service
            .git
            .changelists
            .iter()
            .find(|l| l.name == list_name)
            .unwrap()
            .id
            .clone();
        h.state_mut()
            .app
            .git_prepare_shelf(ShelfAction::MoveToChangelist {
                id,
                paths: vec!["app.js".into()],
            });
        settle(&mut h);
        h.get_by_label("Apply").click();
        settle(&mut h);
        h.state_mut().app.git_ui.commit_open = true;
        h.state_mut().app.git_ui.staging = false;
        h.state_mut().app.git_ui.selected.clear();
        h.run();
        assert!(
            !h.get_by_label("Include app.js in commit")
                .accesskit_node()
                .is_disabled(),
            "ordinary files in a changelist called {list_name} must remain selectable"
        );
        h.get_by_label("Include app.js in commit").click();
        h.run();
        assert!(h.state().app.git_ui.selected.contains(Path::new("app.js")));
        h.get_by_label("Git file app.js").click_secondary();
        h.run();
        h.get_by_label("Show Diff").click();
        settle(&mut h);
        let diff = h.state().app.service.git.diff.as_ref().unwrap();
        assert_eq!(
            diff.side,
            DiffSide::Working,
            "changelist label must not select index mode"
        );
        assert!(String::from_utf8_lossy(&diff.bytes).contains("real working layer"));
        assert_eq!(cli(&root, &["show", ":app.js"]), "real staged layer\n");
    }
}

#[test]
fn scm_open_file_button_opens_own_repository_without_stage_or_dirty_buffer_loss() {
    let (dir, first, mut h) = fixture();
    let second = dir.path().join("second");
    fs::create_dir(&second).unwrap();
    cli(&second, &["init", "-b", "main"]);
    fs::write(second.join("app.js"), "const second = 2;\n").unwrap();
    cli(&second, &["add", "app.js"]);
    cli(
        &second,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-m",
            "second",
        ],
    );
    fs::write(second.join("app.js"), "const second = 3;\n").unwrap();
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
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenFile(first.join("app.js")))
        .unwrap();
    settle(&mut h);
    let doc = h.state().app.service.active_document().unwrap();
    let id = doc.id;
    let version = doc.version;
    h.state_mut()
        .app
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "unsaved first 😀\n".into(),
        })
        .unwrap();
    h.state_mut().app.git_open_source_control();
    h.set_size(egui::vec2(1000., 1200.));
    settle(&mut h);
    let indexes = [cli(&first, &["write-tree"]), cli(&second, &["write-tree"])];
    h.get_by_label(&format!("Open File app.js in {}", second.display()))
        .click();
    settle(&mut h);
    assert_eq!(
        h.state().app.service.active_document().unwrap().path,
        second.join("app.js")
    );
    assert_eq!(
        h.state().app.service.active_document().unwrap().text(),
        "const second = 3;\n"
    );
    h.state_mut()
        .app
        .service
        .dispatch(Command::OpenFile(first.join("app.js")))
        .unwrap();
    settle(&mut h);
    assert_eq!(h.state().app.service.active_document().unwrap().id, id);
    assert_eq!(
        h.state().app.service.active_document().unwrap().text(),
        "unsaved first 😀\n"
    );
    assert!(h.state().app.service.active_document().unwrap().is_dirty());
    assert!(!h.state().app.git_ui.diff_open);
    assert!(h.state().app.git_ui.dialog.is_none());
    assert_eq!(
        [cli(&first, &["write-tree"]), cli(&second, &["write-tree"])],
        indexes
    );
    assert_eq!(
        fs::read_to_string(first.join("app.js")).unwrap(),
        "-- original 😀\n"
    );
}

#[test]
fn git_diff_wrap_syntax_resize_and_unified_preserve_real_patch_and_files() {
    use crate::git_diff_view::assert_wrapped_view;
    for scale in [1., 1.25] {
        let (_dir, root, mut h) = fixture_at_scale(scale);
        let long = "ação 😀 日本 long_identifier_".repeat(12);
        let before = format!(
            "<?php\n/**\n * documentação\n */\nfunction sample(): string {{\n\treturn 'old {long}';\n}}\n"
        );
        let after = before.replace("old ", "new ");
        fs::write(root.join("sample.php"), &before).unwrap();
        cli(&root, &["add", "sample.php"]);
        cli(&root, &["commit", "-m", "PHP fixture"]);
        fs::write(root.join("sample.php"), &after).unwrap();
        h.state_mut().app.git_send(GitCommand::Refresh);
        settle(&mut h);
        let index = cli(&root, &["write-tree"]);
        h.state_mut()
            .app
            .git_open_diff("sample.php".into(), DiffSide::Working);
        settle(&mut h);
        let patch = h
            .state()
            .app
            .git_ui
            .diff_data
            .as_ref()
            .unwrap()
            .text
            .clone();
        assert!(patch.contains(&format!("-\treturn 'old {long}';")));
        assert!(!h.state().app.git_ui.diff_no_wrap);
        assert_wrapped_view(&h.state().app.git_ui.diff_view, true);
        let capture = |h: &mut GitHarness, name: &str| {
            if let Some(out) = std::env::var_os("CODISSEU_GIT_DIFF_CAPTURES") {
                let out = PathBuf::from(out);
                fs::create_dir_all(&out).unwrap();
                h.render()
                    .unwrap()
                    .save(out.join(format!("{name}-{scale}.png")))
                    .unwrap();
            }
        };
        capture(&mut h, "side");
        h.set_size(egui::vec2(760., 700.));
        h.run();
        assert_wrapped_view(&h.state().app.git_ui.diff_view, true);
        capture(&mut h, "compact");
        h.get_by_label("Quebrar linhas").click();
        h.run();
        assert!(h.state().app.git_ui.diff_no_wrap);
        capture(&mut h, "horizontal");
        h.get_by_label("Quebrar linhas").click();
        h.state_mut().app.git_ui.diff_unified = true;
        h.run();
        assert_wrapped_view(&h.state().app.git_ui.diff_view, true);
        capture(&mut h, "unified");
        h.get_by_label("Copy Patch").click();
        h.run();
        assert_eq!(h.state().app.git_ui.diff_data.as_ref().unwrap().text, patch);
        assert_eq!(fs::read_to_string(root.join("sample.php")).unwrap(), after);
        assert_eq!(cli(&root, &["write-tree"]), index);
        h.get_by_label("Close Diff").click();
        h.run();
        assert!(!h.state().app.git_ui.diff_open);
    }
}

#[test]
fn git_diff_large_patch_shapes_viewport_and_staged_open_file_has_distinct_control() {
    let (_dir, root, mut h) = fixture();
    let staged = "const staged = 1;\n";
    fs::write(root.join("app.js"), staged).unwrap();
    cli(&root, &["add", "app.js"]);
    fs::write(
        root.join("app.js"),
        "const changed = 'ação 😀';\n".repeat(6000),
    )
    .unwrap();
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_source_control();
    settle(&mut h);
    assert!(h.query_by_label("Open File staged app.js").is_some());
    assert!(h.query_by_label("Open File app.js").is_some());
    let index = cli(&root, &["write-tree"]);
    h.state_mut()
        .app
        .git_open_diff("app.js".into(), DiffSide::Working);
    settle(&mut h);
    assert!(
        h.state().app.git_ui.diff_view.visible_rows < 100,
        "large patch must construct only the viewport"
    );
    assert!(h.state().app.git_ui.diff_view.visible_rows > 0);
    assert_eq!(cli(&root, &["write-tree"]), index);
    assert_eq!(cli(&root, &["show", ":app.js"]), staged);
}
