use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn fixture(text: &str) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    fixture_language(text, "ts")
}
fn fixture_language(
    text: &str,
    extension: &str,
) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("main.{extension}"));
    fs::write(&path, text).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut h = Harness::builder()
        .with_step_dt(1. / 60.)
        .with_size([1280., 800.])
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    h.step();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h)
}
fn key(h: &mut Harness<'_, CodisseuApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.step();
    h.step();
}
fn selection(h: &Harness<'_, CodisseuApp>) -> Selection {
    h.state().service.active_document().unwrap().selection()
}
fn text<'a>(h: &'a Harness<'_, CodisseuApp>) -> &'a str {
    h.state().service.active_document().unwrap().text()
}
fn count(h: &Harness<'_, CodisseuApp>) -> usize {
    h.state().editor_metrics().selection_count
}
fn select(h: &mut Harness<'_, CodisseuApp>, a: usize, b: usize) {
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection { anchor: a, head: b },
        })
        .unwrap();
    h.step();
}
#[test]
fn clicking_elsewhere_after_word_selection_keeps_typing_at_the_new_caret() {
    for batch in [0, 1, 2] {
        for enter in [false, true] {
            let original = "<?php\n$variable = $variable;\n// end";
            let (_dir, mut h) = fixture_language(original, "php");
            let origin = h.get_by_role(Role::MultilineTextInput).rect().left_top();
            let word = origin + egui::vec2(25., 32.);
            h.event(egui::Event::PointerMoved(word));
            h.step();
            for _ in 0..2 {
                for pressed in [true, false] {
                    h.event(egui::Event::PointerButton {
                        pos: word,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    });
                    if batch != 2 {
                        h.step();
                    }
                }
            }
            if batch != 2 {
                assert_eq!(selection(&h).range(), 7..15);
            }
            // Beyond the short last line: hit testing must place the caret at EOF.
            let destination = origin + egui::vec2(240., 54.);
            h.event(egui::Event::PointerMoved(destination));
            if batch != 2 {
                h.step();
            }
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: destination,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                });
                if batch == 0 {
                    h.step();
                }
            }
            if enter {
                h.key_press(Key::Enter);
            } else {
                h.event(egui::Event::Text("X".into()));
            }
            h.step();
            h.step();
            let expected = format!("{original}{}", if enter { "\n" } else { "X" });
            assert_eq!(text(&h), expected, "batch={batch}, enter={enter}");
            assert_eq!(selection(&h), Selection::caret(expected.chars().count()));
            assert_eq!(count(&h), 1);
        }
    }
}

#[test]
fn double_click_in_separate_native_press_and_release_frames_selects_unicode_word() {
    let (_dir, mut h) = fixture("ação weather fim");
    let pos = h.get_by_role(Role::MultilineTextInput).rect().left_top() + egui::vec2(15., 8.);
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
    }
    assert_eq!(selection(&h), Selection { anchor: 0, head: 4 });
    h.event(egui::Event::Text("X".into()));
    h.step();
    assert_eq!(text(&h), "X weather fim");
}
#[test]
fn ctrl_d_selects_next_wraps_skips_duplicates_and_edits_with_atomic_undo_redo() {
    let original = "ação 😀 ação\r\nação";
    let (_dir, mut h) = fixture(original);
    select(&mut h, 9, 9);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(selection(&h).range(), 7..11);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(count(&h), 2);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(count(&h), 3);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(count(&h), 3);
    h.event(egui::Event::Text("雨".into()));
    h.step();
    assert_eq!(text(&h), "雨 😀 雨\r\n雨");
    assert_eq!(count(&h), 3);
    h.event(egui::Event::Text("x".into()));
    h.step();
    assert_eq!(text(&h), "雨x 😀 雨x\r\n雨x");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "雨 😀 雨\r\n雨");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), original);
    assert_eq!(count(&h), 3);
    key(&mut h, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    assert_eq!(text(&h), "雨 😀 雨\r\n雨");
    key(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(text(&h), " 😀 \r\n");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), "雨 😀 雨\r\n雨");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(count(&h), 1);
}
#[test]
fn literal_phrase_copy_cut_paste_and_external_edit_invalidate_secondary_selections() {
    let original = "e\u{301}lan tail e\u{301}lan tail";
    let (_dir, mut h) = fixture(original);
    select(&mut h, 0, 10);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(count(&h), 2);
    h.event(egui::Event::Copy);
    h.step();
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c,
        egui::OutputCommand::CopyText(t) if t == "e\u{301}lan tail\ne\u{301}lan tail"))
    );
    h.event(egui::Event::Cut);
    h.step();
    assert_eq!(text(&h), " ");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), original);
    assert_eq!(count(&h), 2);
    h.event(egui::Event::Paste("😀".into()));
    h.step();
    assert_eq!(text(&h), "😀 😀");
    select(&mut h, 0, 0);
    assert_eq!(count(&h), 1);
    h.event(egui::Event::Text("X".into()));
    h.step();
    assert_eq!(text(&h), "X😀 😀");
}
#[test]
fn large_document_scan_remains_incremental_and_batch_undo_preserves_untouched_gap() {
    let source = format!("needle {} needle", "x".repeat(2 * 1024 * 1024));
    let (_dir, mut h) = fixture(&source);
    select(&mut h, 0, 6);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(count(&h), 1); // per-frame scan budget cannot traverse 2 MiB at once
    let until = Instant::now() + Duration::from_secs(20);
    while count(&h) != 2 {
        h.step();
        assert!(Instant::now() < until);
    }
    h.event(egui::Event::Text("N".into()));
    h.step();
    assert_eq!(text(&h), source.replace("needle", "N"));
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(text(&h), source);
}

#[test]
fn sparse_batch_undo_redo_maps_passive_view_selection_between_occurrences() {
    let (_dir, mut h) = fixture("ação middle ação");
    select(&mut h, 5, 11);
    let left = h.state().service.active_view().unwrap();
    let s = &mut h.state_mut().service;
    s.dispatch(Command::CreateView).unwrap();
    let right = s.document_views()[1].id;
    s.dispatch(Command::ActivateView { view: right }).unwrap();
    let d = s.active_document().unwrap();
    let (id, version) = (d.id, d.version);
    s.dispatch(Command::SetSelection {
        id,
        expected_version: version,
        selection: Selection { anchor: 0, head: 4 },
    })
    .unwrap();
    s.dispatch(Command::EditBatchTransaction {
        id,
        expected_version: version,
        deltas: vec![0..4, 12..16]
            .into_iter()
            .map(|range| codisseu_core::TextDelta {
                range,
                insert: "x".into(),
            })
            .collect(),
        before: Selection { anchor: 0, head: 4 },
        after: Selection::caret(1),
    })
    .unwrap();
    assert_eq!(
        s.view(left).unwrap().selection(),
        Selection { anchor: 2, head: 8 }
    );
    assert_eq!(s.active_document().unwrap().text(), "x middle x");
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(
        s.view(left).unwrap().selection(),
        Selection {
            anchor: 5,
            head: 11
        }
    );
    assert_eq!(s.active_document().unwrap().text(), "ação middle ação");
    s.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(
        s.view(left).unwrap().selection(),
        Selection { anchor: 2, head: 8 }
    );
}
