//! Soft wrapping is a view projection: the real native widget must expose every
//! continuation while keeping physical source lines, scalar indices and bytes.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;

fn settle(h: &mut H) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        let metrics = h.state().editor_metrics();
        if !h.state().service.is_busy()
            && !metrics.folding_pending
            && !metrics.wrap_pending
            && !metrics.layout_pending
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "editor did not settle: {metrics:?}"
        );
        thread::sleep(Duration::from_millis(2));
    }
    h.step();
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
    assert!(
        h.state().editor_metrics().layout_error.is_none(),
        "{:?}",
        h.state().editor_metrics()
    );
}

fn fixture(name: &str, source: &str, scale: f32) -> (tempfile::TempDir, PathBuf, H) {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    let path = project.join(name);
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service.dispatch(Command::OpenWorkspace(project)).unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([980., 760.])
        .with_pixels_per_point(scale)
        .with_max_steps(80)
        .with_step_dt(1. / 60.)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, path, h)
}

fn key(h: &mut H, mut modifiers: Modifiers, key: Key) {
    modifiers.command = modifiers.ctrl;
    h.key_press_modifiers(modifiers, key);
    h.step();
    h.step();
}
fn source(h: &H) -> String {
    h.state()
        .service
        .active_document()
        .unwrap()
        .snapshot()
        .to_string()
}
fn selection(h: &H) -> Selection {
    h.state().service.active_document().unwrap().selection()
}
fn select(h: &mut H, selection: Selection) {
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection,
        })
        .unwrap();
    h.step();
    h.step();
}
fn click(h: &mut H, pos: egui::Pos2) {
    h.hover_at(pos);
    h.step();
    h.drag_at(pos);
    h.step();
    h.drop_at(pos);
    h.step();
    h.step();
}
fn capture(h: &mut H, name: &str, scale: f32) {
    let Some(out) = std::env::var_os("CODISSEU_WRAP_CAPTURES") else {
        return;
    };
    let out = PathBuf::from(out);
    fs::create_dir_all(&out).unwrap();
    h.event(egui::Event::PointerGone);
    h.step();
    let name = format!("{name}-{}", (scale * 100.) as u32);
    h.render()
        .unwrap()
        .save(out.join(format!("{name}.png")))
        .unwrap();
    fs::write(
        out.join(format!("{name}.json")),
        serde_json::to_vec_pretty(h.state().editor_metrics()).unwrap(),
    )
    .unwrap();
}

fn assert_wrapped_words_remain_whole(h: &H) {
    let snapshot = h.state().service.active_document().unwrap().snapshot();
    let metrics = h.state().editor_metrics();
    let width = metrics
        .wrap_row_rects
        .first()
        .expect("visible wrapped row")
        .rect;
    // The application publishes its editor font through egui's Monospace
    // text style. Letter spacing is a design token rather than an egui style.
    let font = egui::TextStyle::Monospace.resolve(&h.ctx.global_style());
    let tokens: serde_json::Value =
        serde_json::from_str(include_str!("../../../design/tokens.json")).unwrap();
    let letter_spacing = tokens["editor_letter_spacing"].as_f64().unwrap() as f32;
    let long_word_width = h.ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap("b".repeat(50), font, egui::Color32::WHITE)
            .size()
            .x
    }) + 50. * letter_spacing;
    assert!(
        long_word_width < width[2] - width[0] - 1.,
        "the 50-character word must fit intact on an empty visual row"
    );
    for row in &metrics.wrap_row_rects {
        let physical_start = snapshot.line_to_char(row.source_line);
        let physical = snapshot.line(row.source_line);
        let content_end = physical_start
            + physical
                .to_string()
                .trim_end_matches(['\r', '\n'])
                .chars()
                .count();
        if row.end_char > physical_start && row.end_char < content_end {
            let before = snapshot.char(row.end_char - 1);
            let after = snapshot.char(row.end_char);
            assert!(
                before.is_whitespace() || after.is_whitespace(),
                "a word that fits on a row must move intact: source line {}, split {} between {before:?}/{after:?}",
                row.source_line + 1,
                row.end_char - physical_start
            );
        }
    }
}

#[test]
fn markdown_php_javascript_typescript_wrap_by_default_and_resize_without_changing_bytes() {
    for scale in [1., 1.25] {
        // The 50-character token fits a complete visual row, but often does
        // not fit the remaining space after the preceding 20-character token.
        // It must move intact instead of being forcibly split to fill the row.
        let phrase = format!(
            "{} {} ação 😀 e\u{301} texto legível com espaços ",
            "a".repeat(20),
            "b".repeat(50)
        )
        .repeat(8);
        for (name, text) in [
            ("notes.md", format!("# Notas\n\n{phrase}\nFim.\n")),
            (
                "main.php",
                format!("<?php\n$texto = '{phrase}';\necho $texto;\n"),
            ),
            (
                "main.js",
                format!("const texto = `{phrase}`;\nconsole.log(texto);\n"),
            ),
            (
                "main.ts",
                format!("const texto: string = `{phrase}`;\nconsole.log(texto);\n"),
            ),
        ] {
            let (_dir, path, mut h) = fixture(name, &text, scale);
            // Reading scenario: keep Project visible and hide the initially
            // open Database pane through its actual rail control. The viewport
            // remains 980 initially and 900/1440 during the resize checks.
            h.get_by_label("Alternar painel Database").click();
            h.step();
            settle(&mut h);
            h.get_by_role(Role::MultilineTextInput).focus();
            h.step();
            let metrics = h.state().editor_metrics();
            assert!(metrics.soft_wrap, "{name} must wrap by default");
            assert!(
                metrics.visual_rows > metrics.lines && metrics.wrapped_lines > 0,
                "{metrics:?}"
            );
            assert_eq!(
                metrics.lines,
                text.lines().count() + usize::from(text.ends_with('\n'))
            );
            assert_wrapped_words_remain_whole(&h);
            key(&mut h, Modifiers::CTRL, Key::End);
            settle(&mut h);
            assert_eq!(selection(&h).head, text.chars().count());
            assert!(h.state().editor_metrics().caret_in_viewport);
            assert_eq!(
                h.state().editor_metrics().scroll_x,
                0.,
                "EOF must remain readable without horizontal scrolling"
            );
            capture(&mut h, &format!("{name}-wrapped"), scale);
            let before = selection(&h);
            let version = h.state().service.active_document().unwrap().version;
            let narrow = h.state().editor_metrics().visual_rows;
            h.set_size(egui::vec2(1440., 900.));
            settle(&mut h);
            assert!(
                h.state().editor_metrics().visual_rows < narrow,
                "resize must reflow {name}"
            );
            assert!(h.state().editor_metrics().caret_in_viewport);
            h.set_size(egui::vec2(900., 700.));
            settle(&mut h);
            assert!(h.state().editor_metrics().visual_rows >= narrow);
            assert_wrapped_words_remain_whole(&h);
            let d = h.state().service.active_document().unwrap();
            assert_eq!((d.version, d.selection()), (version, before));
            assert!(!d.is_dirty() && !d.can_undo());
            assert_eq!(source(&h), text);
            assert_eq!(fs::read_to_string(path).unwrap(), text);
            capture(&mut h, &format!("{name}-narrow"), scale);
        }
    }
}

#[test]
fn pointer_continuations_unicode_crlf_copy_edit_undo_and_save_keep_canonical_ranges() {
    for scale in [1., 1.25] {
        let text = format!(
            "\t{}\r\nsecond physical line\r\n",
            "ação😀e\u{301}\tword ".repeat(35)
        );
        let (_dir, path, mut h) = fixture("unicode.ts", &text, scale);
        let row = h
            .state()
            .editor_metrics()
            .wrap_row_rects
            .iter()
            .find(|row| {
                row.source_line == 0 && row.start_char > 0 && row.end_char > row.start_char + 4
            })
            .expect("at least one continuation must be visible")
            .clone();
        let point = egui::pos2(row.rect[0] + 1., (row.rect[1] + row.rect[3]) / 2.);
        click(&mut h, point);
        assert_eq!(
            selection(&h),
            Selection::caret(row.start_char),
            "continuation click must map to canonical scalar index"
        );
        assert_eq!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .snapshot()
                .char_to_line(selection(&h).head),
            0
        );
        let range = row.start_char..row.end_char;
        let expected = text
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect::<String>();
        select(
            &mut h,
            Selection {
                anchor: range.end,
                head: range.start,
            },
        );
        h.event(egui::Event::Copy);
        h.step();
        assert!(
            h.output().platform_output.commands.iter().any(
                |cmd| matches!(cmd, egui::OutputCommand::CopyText(value) if value == &expected)
            )
        );
        h.event(egui::Event::Paste("✅ substituído".into()));
        h.step();
        h.step();
        assert_ne!(source(&h), text);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(source(&h), text);
        assert_eq!(
            selection(&h),
            Selection {
                anchor: range.end,
                head: range.start
            }
        );
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Copy);
        h.step();
        assert!(
            h.output()
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(value) if value == &text))
        );
        key(&mut h, Modifiers::CTRL, Key::S);
        settle(&mut h);
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        assert_eq!(h.state().editor_metrics().lines, 3);
        capture(&mut h, "unicode-selection", scale);
    }
}

#[test]
fn vertical_arrows_traverse_visual_rows_and_home_end_reveal_without_horizontal_scroll() {
    let text = format!(
        "{}\nlast physical line",
        "alpha beta gamma delta ".repeat(45)
    );
    let (_dir, _path, mut h) = fixture("navigation.md", &text, 1.);
    let row = h
        .state()
        .editor_metrics()
        .wrap_row_rects
        .iter()
        .find(|row| row.source_line == 0 && row.start_char > 0 && row.end_char > row.start_char + 8)
        .unwrap()
        .clone();
    let initial = row.start_char + 5;
    select(&mut h, Selection::caret(initial));
    key(&mut h, Modifiers::NONE, Key::ArrowDown);
    let next = selection(&h).head;
    assert!(
        next > initial && next < text.find('\n').unwrap(),
        "Down must enter the next visual row of the same physical line"
    );
    key(&mut h, Modifiers::NONE, Key::ArrowUp);
    assert_eq!(
        selection(&h).head,
        initial,
        "Up restores preferred visual column"
    );
    // End owns the trailing affinity of a soft boundary: its scalar index is
    // also the next row's start, but its caret must remain on the original row.
    select(&mut h, Selection::caret(row.end_char - 1));
    let before_end = h.output().platform_output.ime.unwrap().cursor_rect;
    select(&mut h, Selection::caret(initial));
    key(&mut h, Modifiers::NONE, Key::End);
    let end_rect = h.output().platform_output.ime.unwrap().cursor_rect;
    assert_eq!(selection(&h).head, row.end_char);
    assert!(
        end_rect.center().y >= row.rect[1] && end_rect.center().y <= row.rect[3],
        "End must keep the caret on its original visual row: {end_rect:?} vs {:?}",
        row.rect
    );
    assert!(
        end_rect.left() >= before_end.left() - 1.,
        "End must paint after the row's final character, not at the next row origin"
    );
    key(&mut h, Modifiers::NONE, Key::Home);
    let home_rect = h.output().platform_output.ime.unwrap().cursor_rect;
    assert_eq!(selection(&h).head, row.start_char);
    assert!(home_rect.center().y >= row.rect[1] && home_rect.center().y <= row.rect[3]);
    assert_eq!(h.state().editor_metrics().scroll_x, 0.);
    assert!(h.state().editor_metrics().caret_in_viewport);
    assert_eq!(source(&h), text);
}

#[test]
fn folding_and_accepted_diagnostics_keep_physical_line_identity_after_wrap() {
    let text = format!(
        "function outer() {{\n    const message = '{}';\n    return message;\n}}\nlast();\n",
        "readable context ".repeat(40)
    );
    let (_dir, _path, mut h) = fixture("folding.ts", &text, 1.);
    let (path, version) = {
        let d = h.state().service.active_document().unwrap();
        (d.path.clone(), d.version)
    };
    h.state_mut()
        .service
        .language
        .analyzed_versions
        .insert(path.clone(), version);
    h.state_mut().service.language.diagnostics.push(
        serde_json::from_value(serde_json::json!({
            "path":path,"version":version,"start_line":1,"start_character":40,
            "end_line":1,"end_character":100,"message":"Synthetic semantic span",
            "severity":4,"unnecessary":true,"language":"TypeScript"
        }))
        .unwrap(),
    );
    h.step();
    h.step();
    assert!(h.state().editor_metrics().unnecessary_spans > 0);
    let physical_lines = h.state().editor_metrics().lines;
    let expanded_rows = h.state().editor_metrics().visual_rows;
    let label = h
        .get_all_by_role(Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .find(|label| label.starts_with("Fold lines 1–"))
        .expect("outer fold gutter button");
    h.get_by_label(&label).click();
    h.step();
    settle(&mut h);
    assert!(h.state().editor_metrics().folded_lines > 0);
    assert!(h.state().editor_metrics().visual_rows < expanded_rows);
    assert_eq!(h.state().editor_metrics().lines, physical_lines);
    assert_eq!(source(&h), text);
    let label = h
        .get_all_by_role(Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .find(|label| label.starts_with("Unfold lines 1–"))
        .unwrap();
    h.get_by_label(&label).click();
    h.step();
    settle(&mut h);
    assert_eq!(h.state().editor_metrics().visual_rows, expanded_rows);
    assert!(h.state().editor_metrics().unnecessary_spans > 0);
    assert_eq!(source(&h), text);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    capture(&mut h, "diagnostics-and-folds", 1.);
}

#[test]
fn long_line_wrapping_keeps_bounded_cache_and_end_edit_remains_exact() {
    let text = "abcdefgh\t😀e\u{301} ".repeat(8_000);
    let (_dir, path, mut h) = fixture("long.ts", &text, 1.);
    assert!(h.state().editor_metrics().visual_rows > 1);
    assert!(h.state().editor_metrics().cache_bytes <= 4 * 1024 * 1024);
    assert!(h.state().editor_metrics().cached_lines <= 256);
    key(&mut h, Modifiers::CTRL, Key::End);
    settle(&mut h);
    assert_eq!(selection(&h).head, text.chars().count());
    assert_eq!(h.state().editor_metrics().scroll_x, 0.);
    assert!(h.state().editor_metrics().caret_in_viewport);
    h.event(egui::Event::Text("X".into()));
    h.step();
    settle(&mut h);
    assert_eq!(source(&h), format!("{text}X"));
    key(&mut h, Modifiers::CTRL, Key::Z);
    settle(&mut h);
    assert_eq!(source(&h), text);
    assert_eq!(fs::read_to_string(path).unwrap(), text);
    assert!(h.state().editor_metrics().cache_bytes <= 4 * 1024 * 1024);
}

#[test]
fn alt_z_switches_view_projection_without_editing_or_losing_directed_selection() {
    let text = format!("{}\r\nend", "canonical text ".repeat(150));
    let (_dir, _path, mut h) = fixture("toggle.md", &text, 1.25);
    select(
        &mut h,
        Selection {
            anchor: 80,
            head: 10,
        },
    );
    let version = h.state().service.active_document().unwrap().version;
    // Native X11 emits printable text alongside Alt+Z; it must not replace
    // the directed selection while the preference changes.
    // Harness::event processes separate frames; use RawInput for an actual
    // same-frame key/text pair, then exercise the late event below.
    h.input_mut().events.extend([
        egui::Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::ALT,
        },
        egui::Event::Text("z".into()),
        egui::Event::Key {
            key: Key::Z,
            physical_key: Some(Key::Z),
            pressed: false,
            repeat: false,
            modifiers: Modifiers::ALT,
        },
    ]);
    h.step();
    h.step();
    settle(&mut h);
    assert!(!h.state().editor_metrics().soft_wrap);
    assert_eq!(
        h.state().editor_metrics().visual_rows,
        h.state().editor_metrics().lines
    );
    assert_eq!(
        selection(&h),
        Selection {
            anchor: 80,
            head: 10
        }
    );
    key(&mut h, Modifiers::ALT, Key::Z);
    // Winit can deliver the printable event in a later frame as well.
    h.event(egui::Event::Text("Z".into()));
    h.step();
    settle(&mut h);
    assert!(h.state().editor_metrics().soft_wrap);
    assert!(h.state().editor_metrics().visual_rows > h.state().editor_metrics().lines);
    assert_eq!(
        h.state().service.active_document().unwrap().version,
        version
    );
    assert_eq!(
        selection(&h),
        Selection {
            anchor: 80,
            head: 10
        }
    );
    assert_eq!(source(&h), text);
    assert!(!h.state().service.active_document().unwrap().can_undo());
    // A fresh physical key is ordinary typing, even just after the shortcut.
    h.key_press(Key::Z);
    h.event(egui::Event::Text("z".into()));
    h.step();
    assert_eq!(source(&h), format!("{}z{}", &text[..10], &text[80..]));
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(source(&h), text);
    assert_eq!(
        selection(&h),
        Selection {
            anchor: 80,
            head: 10
        }
    );
}

#[test]
fn two_views_reflow_to_independent_widths_and_keep_cursors_scroll_and_shared_text() {
    let text = format!("{}\n", "// readable context ".repeat(12)).repeat(120);
    let (_dir, _path, mut h) = fixture("split.ts", &text, 1.25);
    h.set_size(egui::vec2(1280., 800.));
    settle(&mut h);
    let left = h.state().service.active_view().unwrap();
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.step();
        h.step();
    }
    h.get_by_label("Exibir").click();
    h.step();
    h.step();
    h.get_by_label("Dividir à direita").click();
    h.step();
    settle(&mut h);
    let right = h.state().service.active_view().unwrap();
    assert_ne!(left, right);
    assert_eq!(h.get_all_by_role(Role::MultilineTextInput).count(), 2);
    h.get_all_by_role(Role::MultilineTextInput)
        .nth(1)
        .unwrap()
        .focus();
    h.step();
    key(&mut h, Modifiers::CTRL, Key::End);
    settle(&mut h);
    let right_selection = h.state().service.view(right).unwrap().selection();
    assert!(h.state().editor_view_metrics()[&right].scroll_y > 0.);
    assert_eq!(h.state().editor_view_metrics()[&left].scroll_y, 0.);
    let divider = h
        .get_by_label("Redimensionar grupos do editor")
        .rect()
        .center();
    h.hover_at(divider);
    h.step();
    h.drag_at(divider);
    h.step();
    h.hover_at(divider + egui::vec2(90., 0.));
    h.step();
    h.drop_at(divider + egui::vec2(90., 0.));
    h.step();
    settle(&mut h);
    let metrics = h.state().editor_view_metrics();
    assert_ne!(
        metrics[&left].visual_rows, metrics[&right].visual_rows,
        "the two widths must own independent wrapping projections"
    );
    assert_eq!(metrics[&left].scroll_y, 0.);
    assert!(metrics[&right].scroll_y > 0.);
    assert_eq!(
        h.state().service.view(right).unwrap().selection(),
        right_selection
    );
    let left_selection = h.state().service.view(left).unwrap().selection();
    h.get_all_by_role(Role::MultilineTextInput)
        .nth(1)
        .unwrap()
        .focus();
    h.step();
    key(&mut h, Modifiers::ALT, Key::Z);
    settle(&mut h);
    assert!(
        h.state()
            .editor_view_metrics()
            .values()
            .all(|metrics| !metrics.soft_wrap)
    );
    assert_eq!(
        h.state().service.view(left).unwrap().selection(),
        left_selection
    );
    assert_eq!(
        h.state().service.view(right).unwrap().selection(),
        right_selection
    );
    key(&mut h, Modifiers::ALT, Key::Z);
    settle(&mut h);
    assert!(
        h.state()
            .editor_view_metrics()
            .values()
            .all(|metrics| metrics.soft_wrap)
    );
    assert_eq!(source(&h), text);
    assert_eq!(h.state().service.documents.len(), 1);
    capture(&mut h, "independent-split", 1.25);
}

#[test]
fn menu_and_preference_checkbox_share_state_and_alt_z_does_not_escape_the_modal() {
    let text = "readable source ".repeat(100);
    let (_dir, _path, mut h) = fixture("preferences.ts", &text, 1.);
    select(
        &mut h,
        Selection {
            anchor: 70,
            head: 10,
        },
    );
    let before = selection(&h);
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.step();
        h.step();
    }
    h.get_by_label("Exibir").click();
    h.step();
    h.step();
    h.get_by_label("Quebra automática de linha    Alt+Z")
        .click();
    h.step();
    settle(&mut h);
    assert!(!h.state().service.editor_preferences().soft_wrap);
    key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::S);
    let checkbox = h.get_by_role_and_label(Role::CheckBox, "Quebra automática de linha");
    checkbox.click();
    h.step();
    h.step();
    assert!(h.state().service.editor_preferences().soft_wrap);
    // Space on this real checkbox must toggle the preference, with the editor
    // behind the modal retaining both its source and directed selection.
    h.get_by_role_and_label(Role::CheckBox, "Quebra automática de linha")
        .focus();
    h.step();
    key(&mut h, Modifiers::NONE, Key::Space);
    assert!(!h.state().service.editor_preferences().soft_wrap);
    key(&mut h, Modifiers::ALT, Key::Z);
    assert!(!h.state().service.editor_preferences().soft_wrap);
    assert_eq!(selection(&h), before);
    assert_eq!(source(&h), text);
    capture(&mut h, "preferences", 1.);
    h.get_by_label("Concluído").click();
    h.step();
    settle(&mut h);
    assert!(!h.state().editor_metrics().soft_wrap);
}

#[test]
fn alt_z_in_ai_composer_or_ai_settings_does_not_toggle_the_code_editor() {
    // No declared workspace means AI catalog/model requests cannot start. This
    // isolates shortcut ownership without a real account, process or inference.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("focus.ts");
    let text = "canonical editor text ".repeat(80);
    fs::write(&path, &text).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(80)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    assert!(h.state().service.ai_workspace().is_none());
    h.get_by_label("AI Chat").click();
    h.step();
    h.step();
    h.get_by_role_and_label(Role::MultilineTextInput, "Mensagem AI")
        .focus();
    h.step();
    h.event(egui::Event::Text("synthetic composer draft".into()));
    h.step();
    key(&mut h, Modifiers::ALT, Key::Z);
    assert!(h.state().service.editor_preferences().soft_wrap);
    assert!(
        h.get_by_role_and_label(Role::MultilineTextInput, "Mensagem AI")
            .value()
            .unwrap_or_default()
            .contains("synthetic composer draft")
    );
    h.get_by_label("Configurar AI Chat").click();
    h.step();
    h.step();
    h.get_by_role_and_label(Role::TextInput, "Executável Codex")
        .focus();
    h.step();
    key(&mut h, Modifiers::ALT, Key::Z);
    assert!(h.state().service.editor_preferences().soft_wrap);
    assert!(!h.state().service.ai.is_running());
    assert_eq!(source(&h), text);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}

#[test]
fn resizing_preserves_the_scrolled_source_anchor_inside_one_physical_paragraph() {
    let text = "readable paragraph with meaningful words and stable source positions ".repeat(250);
    let (_dir, _path, mut h) = fixture("anchor.md", &text, 1.);
    let view = h.state().service.active_view().unwrap();
    select(&mut h, Selection::caret(0));
    h.state_mut()
        .service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: [0., 280.],
        })
        .unwrap();
    h.step();
    settle(&mut h);
    assert_eq!(selection(&h), Selection::caret(0));
    assert!(
        !h.state().editor_metrics().caret_in_viewport,
        "the user deliberately scrolled away from the caret"
    );
    let editor = h.get_by_role(Role::MultilineTextInput).rect();
    let first = h
        .state()
        .editor_metrics()
        .wrap_row_rects
        .iter()
        .find(|row| row.source_line == 0 && row.rect[3] > editor.top() + 1.)
        .expect("a visible continuation inside the paragraph");
    let anchor = first.start_char;
    assert!(anchor > 0);
    let version = h.state().service.active_document().unwrap().version;
    h.set_size(egui::vec2(1440., 760.));
    settle(&mut h);
    let editor = h.get_by_role(Role::MultilineTextInput).rect();
    let rows: Vec<_> = h
        .state()
        .editor_metrics()
        .wrap_row_rects
        .iter()
        .filter(|row| row.source_line == 0 && row.rect[3] > editor.top() + 1.)
        .take(2)
        .collect();
    let first = rows
        .first()
        .expect("the paragraph remains visible after reflow");
    let one_row = first.end_char.saturating_sub(first.start_char).max(1);
    let near_first = anchor >= first.start_char.saturating_sub(one_row)
        && anchor <= rows.last().unwrap().end_char;
    assert!(
        near_first,
        "resize must preserve the source anchor within one visual row: old={anchor}, first={}..{}, next_end={}",
        first.start_char,
        first.end_char,
        rows.last().unwrap().end_char
    );
    assert!(
        !h.state().editor_metrics().caret_in_viewport,
        "resize must not reveal a caret the user had scrolled away from"
    );
    assert_eq!(selection(&h), Selection::caret(0));
    assert_eq!(
        h.state().service.active_document().unwrap().version,
        version
    );
    assert_eq!(source(&h), text);
    capture(&mut h, "scrolled-anchor-resized", 1.);
}

#[test]
fn session_restore_preserves_a_scrolled_source_anchor_with_lazy_wrapped_line_heights() {
    let text = (0..70)
        .map(|line| format!("Line {line:02}: {}", "readable source context ".repeat(35)))
        .collect::<Vec<_>>()
        .join("\n");
    let (dir, _path, mut h) = fixture("restore-anchor.md", &text, 1.);
    key(&mut h, Modifiers::CTRL, Key::End);
    settle(&mut h);
    let view = h.state().service.active_view().unwrap();
    let scroll = h.state().editor_metrics().scroll_y;
    assert!(
        scroll > 400.,
        "the fixture must have a real scrolling range"
    );
    h.state_mut()
        .service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: [0., scroll * 0.4],
        })
        .unwrap();
    h.step();
    settle(&mut h);
    let expected_selection = selection(&h);
    assert_eq!(expected_selection, Selection::caret(text.chars().count()));
    assert!(
        !h.state().editor_metrics().caret_in_viewport,
        "the user scrolled away from the EOF caret before saving the session"
    );
    let editor = h.get_by_role(Role::MultilineTextInput).rect();
    let first = h
        .state()
        .editor_metrics()
        .wrap_row_rects
        .iter()
        .find(|row| row.rect[3] > editor.top() + 1.)
        .expect("a visible row before session persistence");
    let anchor = first.start_char;
    let old_line = first.source_line;
    assert!(anchor > 0 && anchor < text.chars().count());
    capture(&mut h, "session-anchor-before", 1.);
    h.state_mut()
        .service
        .dispatch(Command::PersistSession)
        .unwrap();
    settle(&mut h);
    drop(h);

    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::RestoreSession).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "session restoration did not finish"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let mut restored = Harness::builder()
        .with_size([980., 760.])
        .with_pixels_per_point(1.)
        .with_max_steps(80)
        .with_step_dt(1. / 60.)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut restored);
    assert!(restored.state().editor_metrics().soft_wrap);
    assert_eq!(selection(&restored), expected_selection);
    assert!(
        !restored.state().editor_metrics().caret_in_viewport,
        "restoration must respect the user's viewport instead of revealing EOF"
    );
    let editor = restored.get_by_role(Role::MultilineTextInput).rect();
    let rows: Vec<_> = restored
        .state()
        .editor_metrics()
        .wrap_row_rects
        .iter()
        .filter(|row| row.rect[3] > editor.top() + 1.)
        .take(2)
        .collect();
    let first = rows.first().expect("a visible restored source row");
    let one_row = first.end_char.saturating_sub(first.start_char).max(1);
    assert!(
        anchor >= first.start_char.saturating_sub(one_row)
            && anchor <= rows.last().unwrap().end_char,
        "a saved pixel offset must restore its source anchor despite lazy heights: before line={old_line} char={anchor}, after line={} range={}..{}",
        first.source_line,
        first.start_char,
        rows.last().unwrap().end_char
    );
    assert_eq!(source(&restored), text);
    assert!(
        !restored
            .state()
            .service
            .active_document()
            .unwrap()
            .is_dirty()
    );
    capture(&mut restored, "session-anchor-restored", 1.);
}
