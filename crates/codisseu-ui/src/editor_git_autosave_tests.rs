use super::*;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, path::Path, process::Command as Process, thread, time::Instant};
const BASE: &str = "<?php\n$keep = 1;\n$modified = 'before';\n$gap = 1;\n$gap2 = 2;\n$remove = 3;\n$gap3 = 3;\n$gap4 = 4;\n?>\n";
const EDITED: &str = "<?php\n$keep = 1;\n$modified = 'ação 😀';\n$gap = 1;\n$gap2 = 2;\n$gap3 = 3;\n$gap4 = 4;\n$new = 42;\n?>\n";
fn git(p: &Path, args: &[&str]) {
    let o = Process::new("git")
        .arg("-C")
        .arg(p)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn wait(h: &mut Harness<'_, CodisseuApp>, condition: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if condition(h.state()) {
            h.step();
            break;
        }
        assert!(
            Instant::now() < end,
            "timeout: {} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn setup(scale: f32) -> (tempfile::TempDir, PathBuf, Harness<'static, CodisseuApp>) {
    setup_source(scale, BASE)
}
fn setup_source(
    scale: f32,
    source: &str,
) -> (tempfile::TempDir, PathBuf, Harness<'static, CodisseuApp>) {
    let d = tempfile::tempdir().unwrap();
    git(
        d.path(),
        &["-c", "init.templateDir=", "init", "-q", "-b", "main"],
    );
    git(d.path(), &["config", "core.hooksPath", "/dev/null"]);
    git(d.path(), &["config", "user.name", "Fixture"]);
    git(
        d.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    let file = d.path().join("sample.php");
    fs::write(&file, source).unwrap();
    git(d.path(), &["add", "sample.php"]);
    git(d.path(), &["commit", "-qm", "base"]);
    let mut service = AppService::new(d.path().join("profile/session.json"));
    service.dispatch(Command::OpenFile(file.clone())).unwrap();
    let mut app = CodisseuApp::new(service);
    app.show_project = false;
    app.show_database = false;
    app.show_activity = false;
    let mut h = Harness::builder()
        .with_size([1100., 780.])
        .with_pixels_per_point(scale)
        .with_max_steps(40)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
    wait(&mut h, |a| {
        a.service.active_document().is_some() && !a.service.is_busy()
    });
    (d, file, h)
}
fn capture(h: &mut Harness<'_, CodisseuApp>, scale: f32, name: &str, check_colors: bool) {
    let temp = tempfile::tempdir().unwrap();
    let output = std::env::var_os("CODISSEU_GIT_AUTOSAVE_CAPTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp.path().into());
    fs::create_dir_all(&output).unwrap();
    let file = output.join(format!("{name}-{scale}.png"));
    h.render().unwrap().save(&file).unwrap();
    if check_colors {
        let img = image::open(&file).unwrap().to_rgba8();
        for rgb in [
            h.state().tokens.editor_git_added,
            h.state().tokens.editor_git_modified,
            h.state().tokens.editor_git_deleted,
        ] {
            let pixels = img.pixels().filter(|p| p.0[..3] == rgb).count();
            assert!(pixels >= 6, "{rgb:?}: {pixels} pixels");
        }
    }
}
#[test]
fn keyboard_edits_save_without_ctrl_s_and_git_marks_survive_save_and_refresh_index() {
    for scale in [1., 1.25] {
        let (d, file, mut h) = setup(scale);
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(EDITED.into()));
        h.step();
        wait(&mut h, |a| {
            let d = a.service.active_document().unwrap();
            a.service.git_line_changes(d.id, d.version).len() == 3
        });
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.text(), EDITED);
        assert!(doc.is_dirty());
        assert_eq!(fs::read_to_string(&file).unwrap(), BASE);
        capture(&mut h, scale, "unsaved-git-marks", true);
        wait(&mut h, |a| !a.service.active_document().unwrap().is_dirty());
        assert_eq!(fs::read_to_string(&file).unwrap(), EDITED);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(
            h.state()
                .service
                .git_line_changes(doc.id, doc.version)
                .len(),
            3
        );
        capture(&mut h, scale, "autosaved-git-marks", true);
        git(d.path(), &["add", "sample.php"]);
        wait(&mut h, |a| {
            let d = a.service.active_document().unwrap();
            a.service.git_line_changes(d.id, d.version).is_empty()
        });
        capture(&mut h, scale, "staged-no-marks", false);
        git(d.path(), &["reset", "-q", "HEAD", "--", "sample.php"]);
        wait(&mut h, |a| {
            let d = a.service.active_document().unwrap();
            a.service.git_line_changes(d.id, d.version).len() == 3
        });
        // Undo remains a buffer history operation after an automatic save.
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        wait(&mut h, |a| {
            a.service.active_document().unwrap().text() == BASE
        });
        assert_eq!(fs::read_to_string(&file).unwrap(), EDITED);
    }
}
#[test]
fn auto_save_preference_mouse_keyboard_and_disabled_file_stays_dirty() {
    for scale in [1., 1.25] {
        let (_d, file, mut h) = setup(scale);
        h.state_mut().preferences_dialog = true;
        for _ in 0..6 {
            h.step();
        }
        let label = "Salvar arquivos automaticamente";
        h.get_by_label(label).click();
        h.step();
        assert!(!h.state().service.editor_preferences().auto_save);
        h.key_press(Key::Space);
        h.step();
        assert!(h.state().service.editor_preferences().auto_save);
        h.key_press(Key::Space);
        h.step();
        assert!(!h.state().service.editor_preferences().auto_save);
        capture(&mut h, scale, "auto-save-preferences", false);
        h.get_by_label("Concluído").click();
        h.step();
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text(EDITED.into()));
        h.step();
        let end = Instant::now() + Duration::from_millis(1300);
        while Instant::now() < end {
            h.step();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(h.state().service.active_document().unwrap().is_dirty());
        assert_eq!(fs::read_to_string(&file).unwrap(), BASE);
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        wait(&mut h, |a| !a.service.active_document().unwrap().is_dirty());
        assert_eq!(fs::read_to_string(&file).unwrap(), EDITED);
    }
}

#[test]
fn git_stripe_and_php_usage_icon_have_separate_lanes_with_breakpoints_and_folds() {
    for scale in [1., 1.25] {
        for tail in [0, 100, 1000] {
            let base = format!(
                "<?php\nclass Example {{\n    private function original(int $n): int {{\n        return $n;\n    }}\n}}\n{}",
                "\n".repeat(tail)
            );
            let edited = base.replace("function original", "function updated");
            let (_dir, file, mut h) = setup_source(scale, &base);
            h.state_mut()
                .service
                .dispatch(Command::SetAutoSave {
                    enabled: false,
                    delay_ms: 1000,
                })
                .unwrap();
            let doc = h.state().service.active_document().unwrap();
            let (id, expected_version) = (doc.id, doc.version);
            h.state_mut()
                .service
                .dispatch(Command::Edit {
                    id,
                    expected_version,
                    text: edited,
                })
                .unwrap();
            wait(&mut h, |a| {
                let d = a.service.active_document().unwrap();
                !a.php_navigation.pending()
                    && a.editor_metrics().folding_regions >= 2
                    && a.service.git_line_changes(d.id, d.version).len() == 1
            });
            let usage = h.get_by_label("Localizar usos de updated").rect();
            let breakpoint_name = format!("Add breakpoint: {}:3", file.display());
            let breakpoint = h.get_by_label(&breakpoint_name).rect();
            let fold = h.get_by_label("Fold lines 3–5").rect();
            let stripe = h
                .output()
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.fill == color(h.state().tokens.editor_git_modified)
                            && r.rect.y_range().contains(usage.center().y) =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .expect("actual Git rectangle on the declaration row");
            // Deletion triangles extend two points beyond the stripe. Reserve
            // three more points before any navigation hit target.
            assert!(
                stripe.right() + 5. <= usage.left(),
                "{scale}/{tail}: Git {stripe:?} overlaps PHP {usage:?}"
            );
            assert!(usage.right() + 1. <= breakpoint.left());
            assert!(breakpoint.right() < fold.left());
            h.get_by_label(&breakpoint_name).click();
            h.step();
            h.step();
            assert!(
                h.query_by_label(&format!("Remove breakpoint: {}:3", file.display()))
                    .is_some()
            );
            h.get_by_label("Fold lines 3–5").click();
            h.step();
            h.step();
            assert!(h.query_by_label("Unfold lines 3–5").is_some());
            h.get_by_label("Unfold lines 3–5").click();
            h.step();
            h.step();
            capture(&mut h, scale, &format!("php-git-lanes-{tail}"), false);
            assert_eq!(fs::read_to_string(&file).unwrap(), base);
        }
    }
}
