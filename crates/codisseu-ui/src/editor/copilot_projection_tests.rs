//! Real native-editor geometry/input, including source text moved by a ghost zone.
#![cfg(unix)]
use super::tests::{request, setup, wait};
use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{fs, path::Path, process::Command as Process};

fn source(h: &mut Harness<'_, CodisseuApp>, text: &str, caret: usize) {
    let doc = h.state().service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: text.into(),
        })
        .unwrap();
    position(h, caret);
}
fn position(h: &mut Harness<'_, CodisseuApp>, caret: usize) {
    let doc = h.state().service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection::caret(caret),
        })
        .unwrap();
    h.step();
    h.run_steps(2);
}
fn text_rect(h: &Harness<'_, CodisseuApp>, prefix: &str) -> egui::Rect {
    h.root()
        .children_recursive()
        .find(|node| {
            let node = node.accesskit_node();
            node.role() == Role::TextRun
                && node.value().is_some_and(|text| text.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("Missing source TextRun {prefix:?}: {:#?}", h.root()))
        .rect()
}
fn click(h: &mut Harness<'_, CodisseuApp>, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.step();
}
fn close(a: f32, b: f32, scale: f32) {
    assert!((a - b).abs() <= 1. / scale, "{a} != {b} at scale {scale}");
}

#[test]
fn copilot_projection_keeps_accessible_source_bounds_and_pointer_clicks_on_moved_suffix() {
    const TEXT: &str = "call(remaining);\nfollowing();\n";
    for scale in [1., 1.25] {
        let (_dir, file, mut h) = setup(scale, "js", "first,\nsecond");
        let disk = fs::read(&file).unwrap();
        source(&mut h, TEXT, 5);
        let original = text_rect(&h, "call(");
        let following = text_rect(&h, "following");
        request(&mut h);
        let suffix = text_rect(&h, "remaining");
        let moved_following = text_rect(&h, "following");
        let prefix = text_rect(&h, "call(");
        let line_height = h.state().tokens.editor_line_height;
        close(prefix.top(), original.top(), scale);
        close(suffix.top(), original.top() + line_height, scale);
        close(moved_following.top(), following.top() + line_height, scale);
        close(
            suffix.left(),
            h.state()
                .editor_bridge
                .metrics
                .copilot_ghost_rects
                .last()
                .unwrap()[2],
            scale,
        );
        assert_eq!(h.state().editor_bridge.metrics.lines, 3);
        assert_eq!(h.state().editor_bridge.metrics.copilot_ghost_rows, 2);
        assert_eq!(h.state().service.active_document().unwrap().text(), TEXT);

        click(&mut h, egui::pos2(suffix.right() - 1., suffix.center().y));
        let view = h.state().service.active_view().unwrap();
        assert_eq!(
            h.state().service.view(view).unwrap().selection(),
            Selection::caret("call(remaining);".len())
        );
        assert!(!h.state().editor_bridge.metrics.copilot_ghost_visible);
        assert_eq!(h.state().service.active_document().unwrap().text(), TEXT);

        position(&mut h, 5);
        request(&mut h);
        let following = text_rect(&h, "following");
        click(
            &mut h,
            egui::pos2(following.left() + 1., following.center().y),
        );
        assert_eq!(
            h.state().service.view(view).unwrap().selection(),
            Selection::caret("call(remaining);\n".len())
        );
        assert_eq!(h.state().service.active_document().unwrap().text(), TEXT);
        assert_eq!(fs::read(&file).unwrap(), disk);
    }
}

fn git(root: &Path, args: &[&str]) {
    let result = Process::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn copilot_projection_moves_git_authorship_paint_and_hit_target_after_real_suffix() {
    const TEXT: &str = "call(remaining);\nfollowing();\n";
    for scale in [1., 1.25] {
        let (dir, file, mut h) = setup(scale, "js", "first,\nsecond");
        fs::write(&file, TEXT).unwrap();
        git(
            dir.path(),
            &["-c", "init.templateDir=", "init", "-b", "main"],
        );
        git(dir.path(), &["add", "sample.js"]);
        git(
            dir.path(),
            &[
                "-c",
                "user.name=Projection Fixture",
                "-c",
                "user.email=projection@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                "Own fixture",
            ],
        );
        h.state_mut()
            .service
            .dispatch(Command::OpenWorkspace(dir.path().to_path_buf()))
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(file.clone()))
            .unwrap();
        wait(&mut h, |a| {
            a.service.active_document().is_some_and(|d| d.path == file) && !a.service.is_busy()
        });
        source(&mut h, TEXT, 5);
        wait(&mut h, |a| {
            let d = a.service.active_document().unwrap();
            a.git_insights.line(&file, d.version, 1).is_some()
        });
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        // A validated context now requests automatically while blame loads.
        // This geometry fixture measures a source-only baseline; keyboard Esc
        // cancellation is exercised independently below and in native runs.
        h.state_mut().copilot_cancel();
        h.state_mut().copilot_ui.due = None;
        h.step();
        let before = h.get_by_label_contains("Autoria da linha 1:").rect();
        request(&mut h);
        let suffix = text_rect(&h, "remaining");
        let after = h.get_by_label_contains("Autoria da linha 1:").rect();
        close(
            after.top(),
            before.top() + h.state().tokens.editor_line_height,
            scale,
        );
        close(after.top(), suffix.top(), scale);
        close(
            after.left(),
            suffix.right() + h.state().tokens.git_authorship_gap,
            scale,
        );
        for rect in &h.state().editor_bridge.metrics.copilot_ghost_rects {
            let rect = egui::Rect::from_min_max(
                egui::pos2(rect[0], rect[1]),
                egui::pos2(rect[2], rect[3]),
            );
            assert!(
                !after.intersect(rect).is_positive(),
                "Authorship must not intercept ghost code"
            );
        }
        assert_eq!(h.state().service.active_document().unwrap().text(), TEXT);
        assert_eq!(fs::read_to_string(&file).unwrap(), TEXT);
    }
}

fn open_copilot_action_palette(h: &mut Harness<'_, CodisseuApp>) {
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::Key {
                key: Key::ShiftLeft,
                physical_key: Some(Key::ShiftLeft),
                pressed,
                repeat: false,
                modifiers: if pressed {
                    Modifiers::SHIFT
                } else {
                    Modifiers::NONE
                },
            });
            h.step();
        }
    }
    h.run_steps(3);
    h.get_by_label("Ações").click();
    h.step();
    h.get_by_label("Pesquisar no projeto").focus();
    h.step();
    h.event(egui::Event::Text("Copilot".into()));
    h.step();
    h.run_steps(3);
}

#[test]
fn copilot_action_palette_accepts_only_next_line_without_saving_or_inline_buttons() {
    for scale in [1., 1.25] {
        let (_dir, file, mut h) = setup(scale, "js", "const result = 42;\nreturn result;");
        let before = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        let disk = fs::read(&file).unwrap();
        request(&mut h);
        open_copilot_action_palette(&mut h);
        assert!(!h.state().editor_bridge.metrics.copilot_ghost_visible);
        h.get_by_label("Copilot: aceitar próxima linha").click();
        h.step();
        h.run_steps(3);
        assert!(h.query_by_label("Pesquisar no projeto").is_none());
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            format!("{before}const result = 42;\n")
        );
        assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
        assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
        assert_eq!(fs::read(&file).unwrap(), disk);
        assert!(h.query_by_label("Aceitar · Tab").is_none());
        h.key_press_modifiers(Modifiers::CTRL, Key::Z);
        h.step();
        assert_eq!(h.state().service.active_document().unwrap().text(), before);
    }
}

#[test]
fn copilot_action_palette_cannot_apply_a_proposal_after_source_version_changes() {
    for scale in [1., 1.25] {
        let (_dir, file, mut h) = setup(scale, "js", "const result = 42;\nreturn result;");
        let disk = fs::read(&file).unwrap();
        request(&mut h);
        open_copilot_action_palette(&mut h);
        assert!(h.query_by_label("Copilot: aceitar próxima linha").is_some());
        let doc = h.state().service.active_document().unwrap();
        let (id, version, changed) = (
            doc.id,
            doc.version,
            format!("{}// changed while palette open", doc.text()),
        );
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: changed.clone(),
            })
            .unwrap();
        h.step();
        h.run_steps(3);
        assert!(h.query_by_label("Copilot: aceitar próxima linha").is_none());
        h.key_press(Key::Enter);
        h.step();
        assert_eq!(h.state().service.active_document().unwrap().text(), changed);
        assert_eq!(fs::read(&file).unwrap(), disk);
    }
}

fn multipart_fixture(scale: f32) -> (tempfile::TempDir, PathBuf, Harness<'static, CodisseuApp>) {
    let (dir, file, mut h) = setup(scale, "php", "unused");
    let text = "<?php\n    $desconto = $this->processarDescontosMarcados()\nfollowing();\n";
    let line = text.lines().nth(1).unwrap();
    let program = PathBuf::from(&h.state().service.editor_preferences().copilot_program);
    fs::write(program.with_extension("json"), serde_json::json!({"items":[{
        "insertText":format!("{}$fatura);",line.trim_end_matches(')')),
        "range":{"start":{"line":1,"character":0},"end":{"line":1,"character":line.chars().count()}},
        "command":{"command":"github.copilot.didAcceptCompletionItem","arguments":["multipart"]}
    }]}).to_string()).unwrap();
    // The peer reads its options once at startup.
    h.state_mut().copilot_ui.client = None;
    h.state_mut().copilot_ui.attempted = None;
    source(&mut h, text, text.find(')').unwrap());
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.authenticated)
    });
    (dir, file, h)
}

#[test]
fn copilot_multipart_inline_preserves_real_suffix_bounds_tab_undo_and_escape() {
    for scale in [1., 1.25] {
        let (_dir, file, mut h) = multipart_fixture(scale);
        let before = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        let disk = fs::read(&file).unwrap();
        let following = text_rect(&h, "following");
        request(&mut h);
        assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
        let real_close = text_rect(&h, ")");
        let ghost = h
            .query_by_label("Sugestão Copilot: $fatura);")
            .unwrap()
            .rect();
        assert!(real_close.left() > ghost.left());
        assert!(real_close.right() < ghost.right());
        close(text_rect(&h, "following").top(), following.top(), scale);
        assert_eq!(h.state().service.active_document().unwrap().text(), before);
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            before.replace("Marcados()", "Marcados($fatura);")
        );
        h.key_press_modifiers(Modifiers::CTRL, Key::Z);
        h.step();
        assert_eq!(h.state().service.active_document().unwrap().text(), before);
        request(&mut h);
        h.key_press(Key::Escape);
        h.step();
        assert!(!h.state().editor_bridge.metrics.copilot_ghost_visible);
        assert_eq!(h.state().service.active_document().unwrap().text(), before);
        assert_eq!(fs::read(&file).unwrap(), disk);
    }
}

#[test]
fn copilot_multipart_typing_and_partial_word_keep_final_edit_without_duplicate_parenthesis() {
    for typed in [true, false] {
        let (_dir, _, mut h) = multipart_fixture(1.);
        let before = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        request(&mut h);
        if typed {
            h.event(egui::Event::Text("$fa".into()));
            h.step();
        } else {
            h.state_mut().copilot_accept_part(false);
            h.step();
        }
        h.step();
        assert!(
            h.state().editor_bridge.metrics.copilot_ghost_visible,
            "typed={typed}"
        );
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            before.replace("Marcados()", "Marcados($fatura);")
        );
    }
}

#[test]
fn copilot_automatic_restarts_after_selection_navigation_and_delayed_document_edits() {
    for extension in ["php", "sql", "md", "js", "ts", "unknown"] {
        for case in ["selection", "navigation", "delayed_edit"] {
            let (_dir, _, mut h) = setup(1., extension, "return 42;");
            request(&mut h);
            h.key_press(Key::Escape);
            h.step();
            let doc = h.state().service.active_document().unwrap();
            let (id, version, len) = (doc.id, doc.version, doc.text().chars().count());
            match case {
                "selection" => {
                    h.state_mut()
                        .service
                        .dispatch(Command::SetSelection {
                            id,
                            expected_version: version,
                            selection: Selection {
                                anchor: 0,
                                head: len,
                            },
                        })
                        .unwrap();
                    h.step();
                    assert!(h.state().copilot_ui.observed.is_none());
                    h.event(egui::Event::Text("<?php\n".into()));
                    h.step();
                }
                "navigation" => {
                    h.key_press(Key::ArrowLeft);
                    h.step();
                }
                _ => {
                    h.state_mut()
                        .service
                        .dispatch(Command::Edit {
                            id,
                            expected_version: version,
                            text: "<?php\n// asynchronous LSP edit\n".into(),
                        })
                        .unwrap();
                    h.step();
                }
            }
            assert!(
                h.state().copilot_ui.due.is_some()
                    || h.state().copilot_ui.client.as_ref().unwrap().busy,
                "No automatic trigger after {case}"
            );
            wait(&mut h, |a| a.editor_bridge.metrics.copilot_ghost_visible);
        }
    }
}

#[test]
fn copilot_automatic_requests_after_editing_keys_paste_unicode_and_reopening_a_document() {
    for action in [
        "text",
        "unicode",
        "paste",
        "enter",
        "delete",
        "backspace",
        "tab",
        "open",
    ] {
        let (dir, _, mut h) = setup(1., "php", "return 42;");
        source(&mut h, "<?php\ncall();\n", 8);
        request(&mut h);
        h.key_press(Key::Escape);
        h.step();
        match action {
            "text" => h.event(egui::Event::Text("x".into())),
            "unicode" => h.event(egui::Event::Text("ação".into())),
            "paste" => h.event(egui::Event::Paste("$value".into())),
            "enter" => h.key_press(Key::Enter),
            "delete" => h.key_press(Key::Delete),
            "backspace" => h.key_press(Key::Backspace),
            "tab" => h.key_press(Key::Tab),
            _ => {
                let path = dir.path().join("another.php");
                fs::write(&path, "<?php\n// Another document\n").unwrap();
                h.state_mut()
                    .service
                    .dispatch(Command::OpenFile(path.clone()))
                    .unwrap();
                wait(&mut h, |a| {
                    a.service.active_document().is_some_and(|d| d.path == path)
                });
                h.get_by_role(Role::MultilineTextInput).focus();
            }
        }
        h.step();
        wait(&mut h, |a| a.editor_bridge.metrics.copilot_ghost_visible);
    }
}

#[test]
fn copilot_escape_cancels_debounce_until_a_new_context_change() {
    let (_dir, _, mut h) = setup(1., "php", "return 42;");
    request(&mut h);
    h.key_press(Key::Escape);
    h.step();
    h.key_press(Key::ArrowLeft);
    h.step();
    assert!(h.state().copilot_ui.due.is_some());
    h.key_press(Key::Escape);
    h.step();
    assert!(h.state().copilot_ui.due.is_none());
    h.run_steps(50);
    assert!(!h.state().editor_bridge.metrics.copilot_ghost_visible);
    h.key_press(Key::ArrowRight);
    h.step();
    wait(&mut h, |a| a.editor_bridge.metrics.copilot_ghost_visible);
}

#[test]
fn copilot_arbitrary_preserved_spans_project_across_newlines_and_accept_atomically() {
    for scale in [1., 1.25] {
        let (dir, file, mut h) = setup(scale, "js", "unused");
        const SOURCE: &str = "f($a,$b)\nfollowing();\n";
        const TARGET: &str = "f($a,\n    1+$b);";
        let program = PathBuf::from(&h.state().service.editor_preferences().copilot_program);
        fs::write(program.with_extension("json"),serde_json::json!({"items":[{"insertText":TARGET,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":8}}}]}).to_string()).unwrap();
        h.state_mut().copilot_ui.client = None;
        h.state_mut().copilot_ui.attempted = None;
        source(&mut h, SOURCE, 2);
        wait(&mut h, |a| {
            a.copilot_ui
                .client
                .as_ref()
                .is_some_and(|c| c.authenticated)
        });
        h.state_mut().copilot_cancel();
        h.state_mut().copilot_ui.due = None;
        h.step();
        let following = text_rect(&h, "following");
        request(&mut h);
        let first = text_rect(&h, "$a,");
        let second = text_rect(&h, "$b)");
        assert!(second.top() > first.top());
        close(
            text_rect(&h, "following").top(),
            following.top() + h.state().tokens.editor_line_height,
            scale,
        );
        let second_caret = second.left() + second.width() * 0.25;
        click(&mut h, egui::pos2(second_caret, second.center().y));
        let sel = h
            .state()
            .service
            .view(h.state().service.active_view().unwrap())
            .unwrap()
            .selection();
        assert!(
            (5..=7).contains(&sel.head),
            "Click on preserved $b must map to source, got {sel:?}"
        );
        position(&mut h, 2);
        request(&mut h);
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            format!("{TARGET}\nfollowing();\n")
        );
        h.key_press_modifiers(Modifiers::CTRL, Key::Z);
        h.step();
        assert_eq!(h.state().service.active_document().unwrap().text(), SOURCE);
        assert_eq!(fs::read(&file).unwrap(), b"// owned fixture\n");
        drop(dir);
    }
}

#[test]
fn copilot_partial_acceptance_consumes_retained_source_once_and_keeps_final_result() {
    let (_dir, _, mut h) = setup(1., "js", "unused");
    let source_text = "f($a,$b)\n";
    let target = "f($a, 1+$b);";
    let program = PathBuf::from(&h.state().service.editor_preferences().copilot_program);
    fs::write(program.with_extension("json"),serde_json::json!({"items":[{"insertText":target,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":8}}}]}).to_string()).unwrap();
    h.state_mut().copilot_ui.client = None;
    h.state_mut().copilot_ui.attempted = None;
    source(&mut h, source_text, 2);
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.authenticated)
    });
    request(&mut h);
    h.state_mut().copilot_accept_part(false);
    h.run_steps(3);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "f($a, $b)\n"
    );
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        format!("{target}\n")
    );
    h.key_press_modifiers(Modifiers::CTRL, Key::Z);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "f($a, $b)\n"
    );
}
