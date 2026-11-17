use codisseu_core::{AppService, Command, EditKind, Selection, TextDelta};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            h.step();
            return;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
}
fn fixture(text: &str) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    fixture_named("main.ts", text)
}
fn fixture_named(name: &str, text: &str) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(name);
    fs::write(&p, text).unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenFile(p)).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    idle(&mut h);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h)
}
fn key(h: &mut Harness<'_, CodisseuApp>, mut m: Modifiers, k: Key) {
    if m.ctrl {
        m.command = true;
    }
    h.key_press_modifiers(m, k);
    h.step();
    h.step();
}
fn text<'a>(h: &'a Harness<'_, CodisseuApp>) -> &'a str {
    h.state().service.active_document().unwrap().text()
}
fn sel(h: &Harness<'_, CodisseuApp>) -> Selection {
    h.state().service.active_document().unwrap().selection()
}
#[test]
fn pasted_lines_removed_beneath_stationary_hover_keep_editor_and_history_alive() {
    for (newline, delete) in [("\n", false), ("\n", true), ("\r\n", false), ("\r\n", true)] {
        let (dir, mut h) = fixture("");
        h.state_mut()
            .service
            .dispatch(Command::SetAutoSave {
                enabled: false,
                delay_ms: 1000,
            })
            .unwrap();
        let original = format!("préço😀{newline}").repeat(12);
        h.event(egui::Event::Paste(original.clone()));
        h.step();
        h.step();
        let editor = h.get_by_role(Role::MultilineTextInput).rect();
        // The hover belongs to the old final row. Removing that final newline
        // makes it exactly len_lines(), not a valid content row.
        h.hover_at(editor.min + egui::vec2(40., 12.5 * 22.));
        h.step();
        if delete {
            key(&mut h, Modifiers::NONE, Key::ArrowLeft);
        }
        // Native press and release may be delivered in separate frames.
        let delete_key = if delete { Key::Delete } else { Key::Backspace };
        h.key_down(delete_key);
        h.step();
        h.key_up(delete_key);
        h.step();
        let edited = original.strip_suffix(newline).unwrap();
        assert_eq!(text(&h), edited);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(text(&h), original);
        key(&mut h, Modifiers::CTRL, Key::Y);
        assert_eq!(text(&h), edited);
        assert!(!h.state().editor_metrics().layout_pending);
        assert_eq!(fs::read_to_string(dir.path().join("main.ts")).unwrap(), "");
    }
}
#[test]
fn deleting_multiline_selection_rejects_stale_documentation_and_ctrl_link_geometry() {
    for (name, modifiers) in [("main.ts", Modifiers::NONE), ("main.php", Modifiers::CTRL)] {
        for wrap in [false, true] {
            let (dir, mut h) = fixture_named(name, "");
            h.state_mut()
                .service
                .dispatch(Command::SetAutoSave {
                    enabled: false,
                    delay_ms: 1000,
                })
                .unwrap();
            h.state_mut()
                .service
                .dispatch(Command::SetEditorSoftWrap(wrap))
                .unwrap();
            let original = format!("{}préço😀\n", "long_name ".repeat(14)).repeat(8);
            h.event(egui::Event::Paste(original.clone()));
            h.step();
            h.step();
            let metrics = h.state().editor_metrics();
            assert!(!wrap || metrics.visual_rows > metrics.lines);
            let y = (metrics.visual_rows as f32 - 0.5) * 22. - metrics.scroll_y;
            let editor = h.get_by_role(Role::MultilineTextInput).rect();
            let point = editor.min + egui::vec2(40., y);
            assert!(editor.contains(point), "final source row must be visible");
            h.hover_at(point);
            h.step();
            key(&mut h, Modifiers::CTRL, Key::A);
            h.key_down_modifiers(modifiers, Key::Backspace);
            h.step();
            h.key_up_modifiers(Modifiers::NONE, Key::Backspace);
            h.step();
            assert_eq!(text(&h), "");
            key(&mut h, Modifiers::CTRL, Key::Z);
            assert_eq!(text(&h), original);
            key(&mut h, Modifiers::CTRL, Key::Y);
            assert_eq!(text(&h), "");
            assert!(h.state().editor_metrics().layout_error.is_none());
            assert_eq!(fs::read_to_string(dir.path().join(name)).unwrap(), "");
        }
    }
}
#[test]
fn paste_and_backspace_in_one_input_batch_preserve_separate_undo_steps() {
    let original = "const before = '😀';\n".repeat(12);
    let (dir, mut h) = fixture(&original);
    h.state_mut()
        .service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    key(&mut h, Modifiers::CTRL, Key::A);
    let editor = h.get_by_role(Role::MultilineTextInput).rect();
    h.hover_at(editor.min + egui::vec2(40., 12.5 * 22.));
    h.step();
    let pasted = "const preço = 'é😀';\n".repeat(12);
    h.event(egui::Event::Paste(pasted.clone()));
    h.key_down(Key::Backspace);
    h.step();
    h.key_up(Key::Backspace);
    h.step();
    let edited = pasted.strip_suffix('\n').unwrap();
    assert_eq!(text(&h), edited);
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), pasted);
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL, Key::Y);
    key(&mut h, Modifiers::CTRL, Key::Y);
    assert_eq!(text(&h), edited);
    assert!(h.state().editor_metrics().layout_error.is_none());
    assert_eq!(
        fs::read_to_string(dir.path().join("main.ts")).unwrap(),
        original
    );
}
#[test]
fn large_document_virtualizes_scroll_edit_and_undo_without_whole_document_layout() {
    let source = "const weather = 1;\n".repeat(128_000);
    let (_dir, mut h) = fixture(&source);
    assert_eq!(h.state().editor_metrics().lines, 128001);
    assert!(h.state().editor_metrics().visible_end - h.state().editor_metrics().visible_start < 45);
    assert!(h.state().editor_metrics().cached_lines < 100);
    key(&mut h, Modifiers::CTRL, Key::End);
    assert_eq!(sel(&h).head, source.len());
    assert!(h.state().editor_metrics().visible_start > 127950);
    h.event(egui::Event::Text("// end".into()));
    h.step();
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().len_bytes(),
        source.len() + 6
    );
    assert!(h.state().editor_metrics().cache_bytes <= 4 * 1024 * 1024);
    assert!(h.state().editor_metrics().layouts_built < 4);
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert!(text(&h) == source, "Large document content differs");
    key(&mut h, Modifiers::CTRL, Key::Home);
    assert_eq!(sel(&h).head, 0);
    assert_eq!(h.state().editor_metrics().visible_start, 0);
}
#[test]
fn grapheme_crlf_tabs_and_directed_multiline_indentation_preserve_bytes() {
    let original = "😀é👩‍💻🇧🇷\r\n\tfoo\r\nbar";
    let (_dir, mut h) = fixture(original);
    key(&mut h, Modifiers::CTRL, Key::Home);
    for expected in [1, 3, 6, 8, 10] {
        key(&mut h, Modifiers::NONE, Key::ArrowRight);
        assert_eq!(sel(&h).head, expected);
    }
    key(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(text(&h), "😀é👩‍💻🇧🇷\tfoo\r\nbar");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL, Key::A);
    let before = sel(&h);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), "\t😀é👩‍💻🇧🇷\r\n\t\tfoo\r\n\tbar");
    assert_eq!(
        sel(&h),
        Selection {
            anchor: 1,
            head: before.head + 3
        }
    );
    key(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert!(text(&h).starts_with('\t'));
}
#[test]
fn ime_preedit_is_transient_commit_atomic_cancel_and_stale_version_discarded() {
    let (_dir, mut h) = fixture("abc");
    key(&mut h, Modifiers::CTRL, Key::End);
    let version = h.state().service.active_document().unwrap().version;
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "你好".into(),
        active_range_chars: Some(0..2),
    }));
    h.step();
    assert_eq!(text(&h), "abc");
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    assert_eq!(
        h.state().service.active_document().unwrap().version,
        version
    );
    assert!(h.state().editor_metrics().composing);
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("你好".into())));
    h.step();
    assert_eq!(text(&h), "abc你好");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "abc");
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "cancel".into(),
        active_range_chars: None,
    }));
    h.step();
    key(&mut h, Modifiers::NONE, Key::Escape);
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("cancel".into())));
    h.step();
    assert_eq!(text(&h), "abc");
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "stale".into(),
        active_range_chars: None,
    }));
    h.step();
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version, before) = (d.id, d.version, d.selection());
    h.state_mut()
        .service
        .dispatch(Command::EditTransaction {
            id,
            expected_version,
            delta: TextDelta {
                range: 0..0,
                insert: "X".into(),
            },
            kind: EditKind::Atomic,
            before,
            after: Selection::caret(1),
        })
        .unwrap();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("stale".into())));
    h.step();
    assert_eq!(text(&h), "Xabc");
    assert!(!h.state().editor_metrics().composing);
}
#[test]
fn accessibility_uses_real_unicode_and_edits_through_same_history() {
    let (_dir, mut h) = fixture("😀é👩‍💻🇧🇷\r\nend");
    let editor = h.get_by_role(Role::MultilineTextInput);
    assert!(editor.is_focused());
    // AccessKit ReplaceSelectedText travels through AppService like native input.
    editor.set_text_value_accesskit("雨😀\nsecond");
    h.step();
    assert_eq!(text(&h), "雨😀\nsecond");
    key(&mut h, Modifiers::CTRL, Key::A);
    h.get_by_role(Role::MultilineTextInput)
        .replace_selected_text_accesskit("é");
    h.step();
    assert_eq!(text(&h), "é");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "雨😀\nsecond");
    let values = h
        .get_all_by_role(Role::TextRun)
        .filter_map(|n| n.value())
        .collect::<String>();
    assert!(values.contains("雨😀\nsecond"), "{values:?}");
}
#[test]
fn long_line_end_waits_for_measured_layout_then_horizontal_scroll_and_edit_are_exact() {
    let source = "ab\t😀é ".repeat(2000);
    let (_dir, mut h) = fixture(&source);
    // This regression specifically exercises the horizontal, tiled no-wrap mode.
    // Soft wrapping has its own projection and EOF coverage in editor_soft_wrap.
    h.state_mut()
        .service
        .dispatch(Command::SetEditorSoftWrap(false))
        .unwrap();
    h.step();
    key(&mut h, Modifiers::CTRL, Key::End);
    for _ in 0..20 {
        h.step();
    }
    assert!(!h.state().editor_metrics().layout_pending);
    assert!(h.state().editor_metrics().scroll_x > 1000.);
    assert!(h.state().editor_metrics().cache_bytes < 4 * 1024 * 1024);
    h.event(egui::Event::Text("X".into()));
    h.step();
    assert!(
        text(&h) == format!("{source}X"),
        "Long-line insertion differs"
    );
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert!(text(&h) == source, "Large document content differs");
}

#[test]
fn normalized_native_clipboard_retains_crlf_even_after_cutting_entire_document() {
    let original = "// graphemes\r\n😀👩‍💻🇧🇷x\r\n";
    let (_dir, mut h) = fixture(original);
    key(&mut h, Modifiers::CTRL, Key::A);
    h.event(egui::Event::Cut);
    h.step();
    assert_eq!(text(&h), "");
    h.event(egui::Event::Paste(original.replace("\r\n", "\n")));
    h.step();
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL, Key::End);
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(text(&h), format!("{original}\r\n"));
}

#[test]
fn preedit_reflows_suffix_and_multiline_selection_without_exposing_transient_text() {
    let (_dir, mut h) = fixture("ab\r\ncd");
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection { anchor: 1, head: 5 },
        })
        .unwrap();
    h.step();
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "你".into(),
        active_range_chars: Some(0..1),
    }));
    h.step();
    assert_eq!(text(&h), "ab\r\ncd");
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    fn jobs(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.job.text.clone()),
            egui::Shape::Vec(v) => {
                for s in v {
                    jobs(s, out);
                }
            }
            _ => {}
        }
    }
    let mut painted = vec![];
    for shape in &h.output().shapes {
        jobs(&shape.shape, &mut painted);
    }
    assert!(painted.iter().any(|s| s == "a你d"), "{painted:?}");
    let canonical = h
        .get_all_by_role(Role::TextRun)
        .filter_map(|n| n.value())
        .collect::<String>();
    assert!(canonical.contains("ab\r\ncd"));
    assert!(!canonical.contains('你'));
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("你".into())));
    h.step();
    assert_eq!(text(&h), "a你d");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "ab\r\ncd");
    assert_eq!(sel(&h), Selection { anchor: 1, head: 5 });
}

#[test]
fn bundled_cjk_and_emoji_fallbacks_have_real_glyphs_and_preserve_scalar_input() {
    let original = "你好 日本語 한글 😀 👩‍💻 🇧🇷";
    let (_dir, mut h) = fixture(original);
    h.ctx.fonts_mut(|f| {
        for family in [
            egui::FontFamily::Monospace,
            egui::FontFamily::Name("codisseu.mono.bold".into()),
            egui::FontFamily::Name("codisseu.inter.bold".into()),
        ] {
            assert!(f.has_glyphs(&egui::FontId::new(13., family), "你好日本語한글😀👩💻🇧🇷"));
        }
    });
    key(&mut h, Modifiers::CTRL, Key::End);
    key(&mut h, Modifiers::NONE, Key::ArrowLeft);
    h.event(egui::Event::Text("X".into()));
    h.step();
    assert_eq!(text(&h), "你好 日本語 한글 😀 👩‍💻 X🇧🇷");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
}

#[test]
fn status_column_uses_tab_stops_instead_of_scalar_offset() {
    for source in ["//\tX", "//a\tX"] {
        let (_dir, mut h) = fixture(source);
        key(&mut h, Modifiers::CTRL, Key::Home);
        for _ in 0..source.chars().count() - 1 {
            key(&mut h, Modifiers::NONE, Key::ArrowRight);
        }
        // Encoding is a separate control since M3.31; caret columns remain a label.
        h.get_by_label("LF   Ln 1, Col 5");
        h.get_by_label("TypeScript");
        assert_eq!(sel(&h).head, source.chars().count() - 1);
    }
}

#[test]
fn focus_loss_cancels_preedit_and_does_not_apply_a_late_commit_after_refocus() {
    let (_dir, mut h) = fixture("base");
    key(&mut h, Modifiers::CTRL, Key::End);
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "stale".into(),
        active_range_chars: None,
    }));
    h.step();
    if h.query_by_label("Exibir").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Exibir").click();
    h.step();
    h.key_press(Key::Escape);
    h.step();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.event(egui::Event::Ime(egui::ImeEvent::Commit("stale".into())));
    h.step();
    assert_eq!(text(&h), "base");
}

#[test]
fn popup_menu_owns_mouse_and_arrow_keys_without_moving_the_document_caret() {
    let (_dir, mut h) = fixture("abcdef\nsecond line\nthird");
    key(&mut h, Modifiers::CTRL, Key::Home);
    if h.query_by_label("Navegar").is_none() {
        h.get_by_label("Menu principal").click();
        h.run();
    }
    h.get_by_label("Navegar").click();
    h.step();
    h.key_press(Key::ArrowDown);
    h.step();
    assert_eq!(sel(&h), Selection::caret(0));
    h.get_by_label("Ir à definição    F12").click();
    h.step();
    assert_eq!(sel(&h), Selection::caret(0));
    assert_eq!(text(&h), "abcdef\nsecond line\nthird");
}
