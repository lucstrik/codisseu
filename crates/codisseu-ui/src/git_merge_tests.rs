//! Real Git fixtures: review decisions must never write before immutable Apply.
use super::*;

fn settle_full_app(h: &mut Harness<'_, CodisseuApp>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    for _ in 0..3 {
        h.step();
    }
    while h.state().service.is_busy() {
        assert!(Instant::now() < deadline, "full app did not settle");
        thread::sleep(Duration::from_millis(2));
        h.step();
    }
    h.run();
}

#[test]
fn merge_trust_prompt_over_existing_conflict_owns_click_and_preserves_draft() {
    for scale in [1., 1.25] {
        let (dir, root, h, _) = merge_fixture(scale);
        let mut app = h.into_state().app;
        app.styled = false; // A fresh context needs its own font/theme installation.
        let mut h = Harness::builder()
            .with_size([1280., 900.])
            .with_pixels_per_point(scale)
            .with_max_steps(40)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle_full_app(&mut h);
        h.get_by_label("Accept Left change 1").click();
        h.run();
        let draft = h.state().git_ui.merge_ui.result();
        let disk = fs::read(root.join("merge.php")).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        h.state_mut()
            .service
            .dispatch(Command::ConfigureProviders(vec![
                codisseu_core::ProviderConfig {
                    language: codisseu_core::Language::TypeScript,
                    program: dir.path().join("missing-provider"),
                    args: vec!["--stdio".into()],
                },
            ]))
            .unwrap();
        let profile = dir.path().join("policy/provider-policy.json");
        h.state_mut()
            .configure_provider_autostart(profile.clone(), None);
        settle_full_app(&mut h);
        assert!(h.state().service.provider_startup().needs_trust);
        assert!(!h.state().service.provider_startup().trusted);
        h.get_by_label("Trust Project").click();
        settle_full_app(&mut h);
        assert!(
            h.state().service.provider_startup().trusted,
            "Trust click was swallowed by the existing merge modal"
        );
        assert!(h.query_by_label("Trust Project").is_none());
        assert!(h.query_by_label("Accept Left change 1").is_some());
        assert_eq!(h.state().git_ui.merge_ui.result(), draft);
        assert_eq!(fs::read(root.join("merge.php")).unwrap(), disk);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert!(profile.exists());
        assert!(
            h.state().service.language.last_error.is_some(),
            "missing provider must fail explicitly without reopening trust"
        );
    }
}
fn merge_fixture(scale: f32) -> (tempfile::TempDir, PathBuf, GitHarness, String) {
    let (dir, root, mut h) = fixture_at_scale(scale);
    let base = "<?php\nfunction title() { return 'Base'; }\n// shared A\n// shared B\n// shared C\nfunction price() { return 20; }\n// shared D\n// shared E\n// shared F\nfunction timeout() { return 30; }\n// shared G\n// shared H\n// shared I\nfunction audit() { return false; }\n";
    fs::write(root.join("merge.php"), base).unwrap();
    cli(&root, &["add", "merge.php"]);
    cli(&root, &["commit", "-m", "Base"]);
    cli(&root, &["checkout", "-b", "team"]);
    fs::write(
        root.join("merge.php"),
        base.replace("'Base'", "'Team'")
            .replace("return 20", "return 12")
            .replace("return false", "return true"),
    )
    .unwrap();
    cli(&root, &["commit", "-am", "Team"]);
    cli(&root, &["checkout", "main"]);
    fs::write(
        root.join("merge.php"),
        base.replace("'Base'", "'Mine'")
            .replace("return 20", "return 24")
            .replace("return 30", "return 45"),
    )
    .unwrap();
    cli(&root, &["commit", "-am", "Mine"]);
    let merge = Process::new("git")
        .arg("-C")
        .arg(&root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--no-edit",
            "team",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert_eq!(merge.status.code(), Some(1));
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_conflict("merge.php".into());
    settle(&mut h);
    (dir, root, h, base.into())
}
fn result(h: &GitHarness) -> String {
    match &h.state().app.git_ui.dialog {
        Some(GitDialog::Conflict { result, .. }) => result.clone(),
        _ => panic!("conflict editor missing"),
    }
}
fn capture(h: &mut GitHarness, label: &str, scale: f32) {
    if let Some(out) = std::env::var_os("CODISSEU_MERGE_CAPTURES") {
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{label}-{scale}.png")))
            .unwrap();
    }
}
fn merge_menu_action(h: &mut GitHarness, label: &str) {
    if h.query_by_label(label).is_none() {
        h.get_by_label("Merge Settings").click();
        h.run();
    }
    h.get_by_label(label).click();
    h.run();
}
#[test]
fn merge_arrows_ignore_bulk_undo_preview_cancel_and_apply_write_exact_selected_parts() {
    for scale in [1., 1.25] {
        let (_dir, root, mut h, base) = merge_fixture(scale);
        assert_eq!(result(&h), base);
        assert_eq!(h.state().app.git_ui.merge_ui.pending(), 4);
        assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 2);
        let disk = fs::read(root.join("merge.php")).unwrap();
        let index = cli(&root, &["ls-files", "--stage"]);
        assert!(
            h.get_by_label("Apply Changes")
                .accesskit_node()
                .is_disabled()
        );
        capture(&mut h, "initial", scale);
        assert!(
            h.query_by_label("Ctrl+click to resolve conflict").is_none(),
            "merge hints must not cover code when no action is hovered"
        );
        h.get_by_label("Accept Left change 1").click();
        h.run();
        assert_eq!(
            h.state().app.git_ui.merge_ui.conflicts(),
            2,
            "accept alone must preserve other pending choice"
        );
        h.get_by_label("Ignore Right change 1").click();
        h.run();
        assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 1);
        h.get_by_label("Accept Right change 2").click();
        h.run();
        h.get_by_label("Ignore Left change 2").click();
        h.run();
        h.get_by_label("All").click();
        h.run();
        assert_eq!(h.state().app.git_ui.merge_ui.pending(), 0);
        let expected = base
            .replace("'Base'", "'Mine'")
            .replace("return 20", "return 12")
            .replace("return 30", "return 45")
            .replace("return false", "return true");
        assert_eq!(result(&h), expected);
        capture(&mut h, "resolved", scale);
        merge_menu_action(&mut h, "Undo Merge");
        h.run();
        assert_eq!(h.state().app.git_ui.merge_ui.pending(), 2);
        merge_menu_action(&mut h, "Redo Merge");
        h.run();
        assert_eq!(result(&h), expected);
        assert_eq!(fs::read(root.join("merge.php")).unwrap(), disk);
        assert_eq!(cli(&root, &["ls-files", "--stage"]), index);
        h.get_by_label("Apply Changes").click();
        settle(&mut h);
        assert!(matches!(
            h.state().app.git_ui.dialog,
            Some(GitDialog::Preview)
        ));
        h.get_by_label("Cancel").click();
        settle(&mut h);
        assert_eq!(result(&h), expected);
        assert_eq!(h.state().app.git_ui.merge_ui.pending(), 0);
        h.get_by_label("Apply Changes").click();
        settle(&mut h);
        h.get_by_label("Apply").click();
        settle(&mut h);
        assert_eq!(
            fs::read_to_string(root.join("merge.php")).unwrap(),
            expected
        );
        assert!(cli(&root, &["ls-files", "--unmerged"]).is_empty());
        assert!(
            root.join(".git/MERGE_HEAD").exists(),
            "Continue must remain explicit"
        );
    }
}
#[test]
fn merge_accept_both_full_edit_preserve_draft_and_small_window_wrap() {
    let (_dir, root, mut h, base) = merge_fixture(1.);
    h.get_by_label("Accept Left change 1").click();
    h.run();
    h.get_by_label("Accept Right change 1").click();
    h.run();
    assert!(result(&h).contains("'Mine'"));
    assert!(result(&h).contains("'Team'"));
    assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 1);
    merge_menu_action(&mut h, "Undo Merge");
    h.run();
    merge_menu_action(&mut h, "Undo Merge");
    h.run();
    assert_eq!(result(&h), base);
    h.set_size(egui::vec2(900., 680.));
    h.run();
    capture(&mut h, "compact", 1.);
    assert!(h.query_by_label("Close Merge").is_some());
    h.get_by_label("Merge Settings").click();
    h.run();
    assert!(h.query_by_label("Quebrar linhas").is_some());
    h.get_by_label("Merge Settings").click();
    h.run();
    merge_menu_action(&mut h, "Editar resultado completo");
    h.run();
    assert!(
        h.get_by_label("Apply Changes")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_role(Role::MultilineTextInput).click();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text(
        "<?php\n// manual 😀\nreturn 42;\n".into(),
    ));
    h.step();
    assert_eq!(result(&h), "<?php\n// manual 😀\nreturn 42;\n");
    assert!(
        h.get_by_label("Apply Changes")
            .accesskit_node()
            .is_disabled()
    );
    h.run();
    h.get_by_label("Marcar resultado como resolvido").click();
    h.run();
    capture(&mut h, "full-reviewed", 1.);
    assert_eq!(h.state().app.git_ui.merge_ui.pending(), 0);
    h.get_by_label("Apply Changes").click();
    settle(&mut h);
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(result(&h).contains("manual 😀"));
    merge_menu_action(&mut h, "Undo Merge");
    h.run();
    assert_eq!(result(&h), base);
    assert!(!cli(&root, &["ls-files", "--unmerged"]).is_empty());
}

#[test]
fn merge_inline_multiline_paste_and_keyboard_undo_do_not_leave_inserted_lines_or_write() {
    let (_dir, root, mut h, base) = merge_fixture(1.);
    let disk = fs::read(root.join("merge.php")).unwrap();
    h.get_by_label("Merge Result line 2").click();
    h.step();
    h.key_press(Key::Home);
    h.key_press_modifiers(Modifiers::SHIFT, Key::End);
    h.event(egui::Event::Text(
        "function title() { return 'Manual 😀'; }\n// extra pasted line".into(),
    ));
    h.step();
    h.run();
    assert!(result(&h).contains("extra pasted line"));
    assert_eq!(h.state().app.git_ui.merge_ui.pending(), 4);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run();
    assert_eq!(result(&h), base);
    assert_eq!(fs::read(root.join("merge.php")).unwrap(), disk);
}

#[test]
fn merge_ctrl_click_uses_click_modifiers_even_when_control_is_released_in_same_frame() {
    let (_dir, _root, mut h, _base) = merge_fixture(1.);
    let p = h.get_by_label("Accept Right change 1").rect().center();
    let pos = egui::pos2(p.x, p.y);
    h.event_modifiers(egui::Event::PointerMoved(pos), Modifiers::CTRL);
    h.event_modifiers(
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::CTRL,
        },
        Modifiers::CTRL,
    );
    h.event_modifiers(
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::CTRL,
        },
        Modifiers::CTRL,
    );
    h.event_modifiers(egui::Event::PointerMoved(pos), Modifiers::NONE);
    h.step();
    h.run();
    assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 1);
    assert!(result(&h).contains("'Team'"));
    assert!(!result(&h).contains("'Mine'"));
}

#[test]
fn merge_settings_compare_details_nonconflicting_sides_and_delete_cancel_keep_snapshots() {
    let (_dir, root, mut h, base) = merge_fixture(1.);
    let disk = fs::read(root.join("merge.php")).unwrap();
    let index = cli(&root, &["ls-files", "--stage"]);
    h.get_by_label("Left").click();
    h.run();
    assert!(result(&h).contains("return 45"));
    assert!(result(&h).contains("return false"));
    h.get_by_label("Right").click();
    h.run();
    assert!(result(&h).contains("return true"));
    assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 2);
    merge_menu_action(&mut h, "Undo Merge");
    merge_menu_action(&mut h, "Undo Merge");
    assert_eq!(result(&h), base);
    h.get_by_label("Compare Contents").click();
    h.run();
    h.get_by_label("Base ↔ Left").click();
    h.run();
    assert!(h.query_by_label("Close Comparison").is_some());
    h.get_by_label("Close Comparison").click();
    h.run();
    assert_eq!(result(&h), base);
    h.get_by_label("Show Left Details").click();
    h.run();
    let oid = h
        .state()
        .app
        .service
        .git
        .conflict
        .as_ref()
        .unwrap()
        .ours
        .as_ref()
        .unwrap()
        .oid
        .clone();
    assert!(h.query_by_label(&format!("Git object: {oid}")).is_some());
    h.get_by_label("Show Left Details").click();
    h.run();
    merge_menu_action(&mut h, "Quebrar linhas");
    assert!(h.state().app.git_ui.merge_ui.no_wrap);
    // Checkbox menus remain open until dismissed.
    h.get_by_label("Merge Settings").click();
    h.run();
    merge_menu_action(&mut h, "Quebrar linhas");
    assert!(!h.state().app.git_ui.merge_ui.no_wrap);
    h.get_by_label("Merge Settings").click();
    h.run();
    merge_menu_action(&mut h, "Delete File…");
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert_eq!(result(&h), base);
    assert_eq!(fs::read(root.join("merge.php")).unwrap(), disk);
    assert_eq!(cli(&root, &["ls-files", "--stage"]), index);
}

#[test]
fn merge_simple_conflicts_combine_disjoint_line_edits_then_save_without_staging() {
    let (_dir, root, mut h) = fixture();
    let base = "const options = { timeout: 30, retries: 2 };\r\n";
    fs::write(root.join("simple.js"), base).unwrap();
    cli(&root, &["add", "simple.js"]);
    cli(&root, &["commit", "-m", "Base simple"]);
    cli(&root, &["checkout", "-b", "team"]);
    fs::write(
        root.join("simple.js"),
        base.replace("retries: 2", "retries: 4"),
    )
    .unwrap();
    cli(&root, &["commit", "-am", "Team"]);
    cli(&root, &["checkout", "main"]);
    fs::write(
        root.join("simple.js"),
        base.replace("timeout: 30", "timeout: 45"),
    )
    .unwrap();
    cli(&root, &["commit", "-am", "Mine"]);
    let merge = Process::new("git")
        .arg("-C")
        .arg(&root)
        .args(["merge", "--no-edit", "team"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert_eq!(merge.status.code(), Some(1));
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_conflict("simple.js".into());
    settle(&mut h);
    h.get_by_label("Resolve Simple Conflicts").click();
    h.run();
    let expected = base
        .replace("timeout: 30", "timeout: 45")
        .replace("retries: 2", "retries: 4");
    assert_eq!(result(&h), expected);
    assert_eq!(h.state().app.git_ui.merge_ui.pending(), 0);
    merge_menu_action(&mut h, "Stage as resolved");
    assert!(!h.state().app.git_ui.conflict_stage);
    h.get_by_label("Merge Settings").click();
    h.run();
    h.get_by_label("Save and Close").click();
    settle(&mut h);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("simple.js")).unwrap(),
        expected
    );
    assert!(!cli(&root, &["ls-files", "--unmerged"]).is_empty());
    assert!(h.state().app.git_ui.dialog.is_none());
}

#[test]
fn merge_external_change_after_preview_is_refused_without_overwrite_and_keeps_draft() {
    let (_dir, root, mut h, _base) = merge_fixture(1.);
    h.get_by_label("Accept Left").click();
    h.run();
    let draft = result(&h);
    h.get_by_label("Apply Changes").click();
    settle(&mut h);
    assert!(matches!(
        h.state().app.git_ui.dialog,
        Some(GitDialog::Preview)
    ));
    let external = "<?php\n// external change must survive 😀\n";
    fs::write(root.join("merge.php"), external).unwrap();
    let index = cli(&root, &["ls-files", "--stage"]);
    h.get_by_label("Apply").click();
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(root.join("merge.php")).unwrap(),
        external
    );
    assert_eq!(cli(&root, &["ls-files", "--stage"]), index);
    assert_eq!(result(&h), draft);
    assert!(h.state().app.service.git.error.is_some());
}

#[test]
fn merge_ignore_both_context_exclusive_reset_and_copy_match_real_draft() {
    let (_dir, root, mut h, base) = merge_fixture(1.);
    h.get_by_label("Ignore Left change 1").click();
    h.run();
    h.get_by_label("Ignore Right change 1").click();
    h.run();
    assert_eq!(result(&h), base);
    assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 1);
    h.get_by_label("Merge Result line 2").click_secondary();
    h.run();
    h.get_by_label("Resolve using Right").click();
    h.run();
    assert!(result(&h).contains("'Team'"));
    assert!(!result(&h).contains("'Mine'"));
    h.get_by_label("Merge Result line 2").click_secondary();
    h.run();
    h.get_by_label("Reset Change to Base").click();
    h.run();
    assert_eq!(result(&h), base);
    assert_eq!(h.state().app.git_ui.merge_ui.conflicts(), 2);
    h.get_by_label("Merge Settings").click();
    h.run();
    h.get_by_label("Copy Result").click();
    h.step();
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c,egui::OutputCommand::CopyText(t)if t==&base))
    );
    assert!(!cli(&root, &["ls-files", "--unmerged"]).is_empty());
}

#[test]
fn merge_large_real_conflict_virtualizes_rows_and_preserves_complete_versions() {
    let (_dir, root, mut h) = fixture();
    let source = |tag: &str| {
        (0..6000)
            .map(|n| format!("const line{n} = '{tag} ação 😀';\n"))
            .collect::<String>()
    };
    for (branch, tag) in [("main", "base"), ("team", "theirs"), ("main", "ours")] {
        if branch == "team" {
            cli(&root, &["checkout", "-b", "team"]);
        } else if tag == "ours" {
            cli(&root, &["checkout", "main"]);
        }
        fs::write(root.join("large.js"), source(tag)).unwrap();
        cli(&root, &["add", "large.js"]);
        cli(&root, &["commit", "-m", tag]);
    }
    let merge = Process::new("git")
        .arg("-C")
        .arg(&root)
        .args(["merge", "--no-edit", "team"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert_eq!(merge.status.code(), Some(1));
    h.state_mut().app.git_send(GitCommand::Refresh);
    settle(&mut h);
    h.state_mut().app.git_open_conflict("large.js".into());
    settle(&mut h);
    assert_eq!(result(&h), source("base"));
    assert!(h.state().app.git_ui.merge_ui.visible_rows < 100);
    assert!(h.state().app.git_ui.merge_ui.visible_rows > 0);
    let before = fs::read(root.join("large.js")).unwrap();
    h.get_by_label("Accept Left").click();
    h.run();
    assert_eq!(result(&h), source("ours"));
    assert_eq!(fs::read(root.join("large.js")).unwrap(), before);
}
