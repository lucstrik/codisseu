//! Real editor input, shared versioned buffers and disposable filesystem effects.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
type H = Harness<'static, CodisseuApp>;
fn fixture(name: &str, text: &str) -> (tempfile::TempDir, H) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::write(&path, text).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(40)
        .with_step_dt(1. / 60.)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h)
}
fn settle(h: &mut H) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().service.is_busy() && !h.state().editor_metrics().folding_pending {
            break;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    h.step();
}
fn key(h: &mut H, mut m: Modifiers, k: Key) {
    // Linux egui-winit reports both ctrl and platform command for Control.
    m.command = m.ctrl;
    h.key_press_modifiers(m, k);
    h.step();
    h.step();
}
fn select(h: &mut H, a: usize, b: usize) {
    let d = h.state().service.active_document().unwrap();
    let (id, version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection { anchor: a, head: b },
        })
        .unwrap();
    h.step();
}
fn text(h: &H) -> String {
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
fn chord(h: &mut H, m: Modifiers, k: Key) {
    key(h, Modifiers::CTRL, Key::K);
    key(h, m, k);
}
#[test]
fn move_duplicate_delete_insert_unicode_crlf_are_atomic_and_save_exact() {
    let original = "first\r\n  ação 😀\r\nlast";
    let (dir, mut h) = fixture("code.ts", original);
    select(&mut h, 9, 9);
    key(&mut h, Modifiers::ALT, Key::ArrowUp);
    assert_eq!(text(&h), "  ação 😀\r\nfirst\r\nlast");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    assert_eq!(selection(&h), Selection::caret(9));
    key(
        &mut h,
        Modifiers::CTRL.plus(Modifiers::SHIFT).plus(Modifiers::ALT),
        Key::ArrowDown,
    );
    assert_eq!(text(&h), "first\r\n  ação 😀\r\n  ação 😀\r\nlast");
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::K);
    assert_eq!(text(&h), original);
    select(&mut h, 9, 9);
    key(&mut h, Modifiers::CTRL, Key::Enter);
    assert!(text(&h).contains("ação 😀\r\n  \r\n"));
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL, Key::S);
    settle(&mut h);
    assert_eq!(
        fs::read(dir.path().join("code.ts")).unwrap(),
        original.as_bytes()
    );
}
#[test]
fn multi_cursors_keep_navigation_editing_and_cursor_undo() {
    let (_dir, mut h) = fixture("code.ts", "one\ntwo\nthree");
    select(&mut h, 1, 1);
    key(
        &mut h,
        Modifiers::SHIFT.plus(Modifiers::ALT),
        Key::ArrowDown,
    );
    assert_eq!(h.state().editor_metrics().selection_count, 2);
    key(&mut h, Modifiers::NONE, Key::ArrowRight);
    assert_eq!(h.state().editor_metrics().selection_count, 2);
    h.event(egui::Event::Text("X".into()));
    h.step();
    assert_eq!(text(&h), "onXe\ntwXo\nthree");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "one\ntwo\nthree");
    key(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(h.state().editor_metrics().selection_count, 1);
    key(
        &mut h,
        Modifiers::SHIFT.plus(Modifiers::ALT),
        Key::ArrowDown,
    );
    key(&mut h, Modifiers::CTRL, Key::U);
    assert_eq!(h.state().editor_metrics().selection_count, 1);
}
#[test]
fn all_occurrences_skip_and_line_end_cursors_edit_real_ranges() {
    let (_dir, mut h) = fixture("code.ts", "alpha alpha alpha\nalpha");
    select(&mut h, 0, 5);
    chord(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(selection(&h).range(), 6..11);
    assert_eq!(h.state().editor_metrics().selection_count, 1);
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::L);
    for _ in 0..20 {
        h.step();
    }
    assert_eq!(h.state().editor_metrics().selection_count, 4);
    h.event(egui::Event::Text("β".into()));
    h.step();
    assert_eq!(text(&h), "β β β\nβ");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "alpha alpha alpha\nalpha");
    key(&mut h, Modifiers::NONE, Key::Escape);
    select(&mut h, 0, 23);
    key(&mut h, Modifiers::SHIFT.plus(Modifiers::ALT), Key::I);
    assert_eq!(h.state().editor_metrics().selection_count, 2);
    h.event(egui::Event::Text("!".into()));
    h.step();
    assert_eq!(text(&h), "alpha alpha alpha!\nalpha!");
}
#[test]
fn comments_chords_and_smart_home_preserve_selection_and_do_not_open_git() {
    let original = "    const x = 1;\n    const y = 2;\n";
    let (_dir, mut h) = fixture("code.ts", original);
    select(&mut h, 8, 8);
    key(&mut h, Modifiers::NONE, Key::Home);
    assert_eq!(selection(&h).head, 4);
    key(&mut h, Modifiers::NONE, Key::Home);
    assert_eq!(selection(&h).head, 0);
    select(&mut h, original.chars().count(), 0);
    key(&mut h, Modifiers::CTRL, Key::Slash);
    assert!(text(&h).contains("// const x"));
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    chord(&mut h, Modifiers::CTRL, Key::C);
    let commented = text(&h);
    assert!(commented.contains("// const"));
    chord(&mut h, Modifiers::CTRL, Key::C);
    assert_eq!(text(&h), commented);
    chord(&mut h, Modifiers::CTRL, Key::U);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::A);
    assert!(text(&h).contains("/*"));
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
}
#[test]
fn empty_selection_copy_cut_and_line_paste_keep_exact_text() {
    let (_dir, mut h) = fixture("code.ts", "first\r\nsecond");
    select(&mut h, 2, 2);
    h.event(egui::Event::Copy);
    h.step();
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c,egui::OutputCommand::CopyText(t)if t=="first\r\n"))
    );
    select(&mut h, 9, 9);
    h.event(egui::Event::Paste("first\r\n".into()));
    h.step();
    assert_eq!(text(&h), "first\r\nfirst\r\nsecond");
    key(&mut h, Modifiers::CTRL, Key::Z);
    h.event(egui::Event::Cut);
    h.step();
    assert_eq!(text(&h), "first");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "first\r\nsecond");
}
#[test]
fn fold_compresses_view_not_source_and_reveal_and_undo_are_safe() {
    let original = "function outer() {\n  if (true) {\n    console.log('ação');\n  }\n}\nlast();\n";
    let (dir, mut h) = fixture("code.ts", original);
    assert!(h.state().editor_metrics().folding_regions >= 2);
    chord(&mut h, Modifiers::CTRL, Key::Num0);
    assert!(h.state().editor_metrics().folded_lines >= 4);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::CTRL, Key::S);
    settle(&mut h);
    assert_eq!(
        fs::read_to_string(dir.path().join("code.ts")).unwrap(),
        original
    );
    chord(&mut h, Modifiers::CTRL, Key::J);
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    select(&mut h, 0, 0);
    key(
        &mut h,
        Modifiers::CTRL.plus(Modifiers::SHIFT),
        Key::OpenBracket,
    );
    assert!(h.state().editor_metrics().folded_lines > 0);
    select(
        &mut h,
        original.find("console").unwrap(),
        original.find("console").unwrap(),
    );
    h.step();
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    assert_eq!(text(&h), original);
}
#[test]
fn markdown_split_preview_live_task_and_history_share_one_document() {
    let original =
        "# Notes\n\n**Bold** and `code`.\n\n- [ ] Task\n\n| A | B |\n|---|---|\n| one | two |\n";
    let (dir, mut h) = fixture("README.md", original);
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::V);
    let until = Instant::now() + Duration::from_secs(10);
    while h.query_by_label("Markdown task line 5").is_none() {
        h.step();
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(5));
    }
    h.get_by_label("Markdown task line 5").click();
    h.step();
    assert!(text(&h).contains("- [x] Task"));
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        original
    );
    h.get_by_label("Markdown Source").click();
    h.step();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    chord(&mut h, Modifiers::NONE, Key::V);
    h.step();
    assert!(h.query_by_label("Markdown Split").is_some());
    assert_eq!(h.state().service.documents.len(), 1);
    assert_eq!(h.state().service.document_views().len(), 1);
}

#[test]
fn native_clipboard_events_complete_chords_and_unknown_chord_text_is_consumed() {
    let (_dir, mut h) = fixture("code.ts", "alpha  \nbeta  ");
    select(&mut h, 0, 0);
    key(&mut h, Modifiers::CTRL, Key::K);
    h.input_mut().events.extend([
        egui::Event::Key {
            key: Key::ControlLeft,
            physical_key: Some(Key::ControlLeft),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        },
        egui::Event::ModifiersChanged(Modifiers::CTRL.plus(Modifiers::COMMAND)),
        egui::Event::Copy,
    ]);
    h.step();
    assert_eq!(text(&h), "// alpha  \nbeta  ");
    assert!(
        !h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c, egui::OutputCommand::CopyText(_)))
    );
    chord(&mut h, Modifiers::CTRL, Key::U);
    assert_eq!(text(&h), "alpha  \nbeta  ");
    key(&mut h, Modifiers::CTRL, Key::K);
    h.event(egui::Event::Cut);
    h.step();
    assert_eq!(text(&h), "alpha\nbeta");
    key(&mut h, Modifiers::CTRL, Key::Z);
    key(&mut h, Modifiers::CTRL, Key::K);
    h.event(egui::Event::Key {
        key: Key::M,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.event(egui::Event::Text("m".into()));
    h.step();
    assert_eq!(text(&h), "alpha  \nbeta  ");
}
#[test]
fn multiple_events_after_document_shrinks_use_current_folding_projection() {
    let (_dir, mut h) = fixture("code.ts", "one\ntwo\nthree\nfour\nfive\n");
    select(&mut h, 0, 24);
    h.input_mut().events.push(egui::Event::Cut);
    h.input_mut().events.push(egui::Event::Key {
        key: Key::ArrowDown,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    h.step();
    assert_eq!(text(&h), " ".trim());
    assert_eq!(selection(&h), Selection::caret(0));
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "one\ntwo\nthree\nfour\nfive\n");
}
#[test]
fn shift_tab_multi_selection_never_replaces_the_selected_code() {
    let original = "\talpha\r\n\talpha";
    let (_dir, mut h) = fixture("code.ts", original);
    select(&mut h, 1, 6);
    key(&mut h, Modifiers::CTRL, Key::D);
    assert_eq!(h.state().editor_metrics().selection_count, 2);
    key(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(text(&h), "alpha\r\nalpha");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    key(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(text(&h), "\t\talpha\r\n\t\talpha");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
}
#[test]
fn line_clipboard_distributes_normalized_lines_to_multiple_carets() {
    let original = "a\r\nb";
    let (_dir, mut h) = fixture("code.ts", original);
    select(&mut h, 0, 0);
    key(
        &mut h,
        Modifiers::SHIFT.plus(Modifiers::ALT),
        Key::ArrowDown,
    );
    h.event(egui::Event::Copy);
    h.step();
    assert!(
        h.output()
            .platform_output
            .commands
            .iter()
            .any(|c| matches!(c,egui::OutputCommand::CopyText(t)if t=="a\r\nb\r\n"))
    );
    h.event(egui::Event::Paste("a\nb\n".into()));
    h.step();
    assert_eq!(text(&h), "a\r\na\r\nb\r\nb");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
    h.event(egui::Event::Paste("X\nY".into()));
    h.step();
    assert_eq!(text(&h), "Xa\r\nYb");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), original);
}
#[test]
fn inactive_split_context_actions_bind_to_that_document_not_the_active_neighbour() {
    let (dir, mut h) = fixture("first.ts", "FIRST text");
    let first = h.state().service.active_view().unwrap();
    select(&mut h, 1, 4);
    let first_id = h.state().service.active_document().unwrap().id;
    let second_path = dir.path().join("second.ts");
    fs::write(&second_path, "SECOND\nnext").unwrap();
    h.state_mut().service.dispatch(Command::CreateView).unwrap();
    let second = h.state().service.active_view().unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(second_path.clone()))
        .unwrap();
    settle(&mut h);
    h.state_mut()
        .service
        .dispatch(Command::ActivateView { view: first })
        .unwrap();
    h.step();
    // Context clicks position the caret. Target the first logical line instead
    // of the center of a full-height viewport, which belongs to the final line.
    let pos = h
        .get_by_label(second_path.to_string_lossy().as_ref())
        .rect()
        .left_top()
        + egui::vec2(15., 8.);
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
    h.get_by_label("Recortar    Ctrl+X").click();
    h.step();
    assert_eq!(h.state().service.active_view(), Some(second));
    assert_eq!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == first_id)
            .unwrap()
            .text(),
        "FIRST text"
    );
    assert_eq!(text(&h), "next");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "SECOND\nnext");
}
#[test]
fn fields_keep_their_undo_and_markdown_shortcuts_do_not_capture_paste() {
    let (_dir, mut h) = fixture("README.md", "# Notes\n");
    h.event(egui::Event::Text("X".into()));
    h.step();
    let dirty = text(&h);
    h.get_by_label("Markdown Split").click();
    h.step();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    key(&mut h, Modifiers::CTRL, Key::F);
    h.get_by_role_and_label(Role::TextInput, "Localizar")
        .focus();
    h.step();
    h.event(egui::Event::Text("field".into()));
    h.step();
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), dirty);
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::V);
    h.event(egui::Event::Paste("pasted in field".into()));
    h.step();
    assert!(h.query_by_role(Role::MultilineTextInput).is_some());
    assert_eq!(text(&h), dirty);
}
#[test]
fn unsupported_block_comment_keeps_source_and_sql_modifiers_never_execute_accidentally() {
    let (_dir, mut h) = fixture("script.py", "print('hello')");
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::A);
    assert_eq!(text(&h), "print('hello')");
    assert!(
        h.state()
            .service
            .status
            .contains("não define comentário de bloco")
    );
    let (_dir, mut h) = fixture("query.sql", "select 1;");
    key(&mut h, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Enter);
    assert_eq!(text(&h), "\nselect 1;");
    key(&mut h, Modifiers::CTRL, Key::Z);
    assert_eq!(text(&h), "select 1;");
    key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::Enter);
    assert_eq!(text(&h), "\nselect 1;");
}

#[test]
fn new_markdown_save_as_cancel_create_and_existing_path_are_safe() {
    let (dir, mut h) = fixture("first.ts", "keep");
    let path = dir.path().join("new.md");
    // Ctrl+N has created an in-memory Untitled document since29/10. The path
    // dialog belongs to its first Save, and cancelling must retain that buffer.
    key(&mut h, Modifiers::CTRL, Key::N);
    assert!(h.state().service.active_document().unwrap().is_untitled());
    assert_eq!(h.state().service.documents.len(), 2);
    assert!(!path.exists());
    let draft = "# New 🌧\n\n**Draft**";
    h.event(egui::Event::Text(draft.into()));
    h.step();
    assert_eq!(text(&h), draft);
    key(&mut h, Modifiers::CTRL, Key::S);
    h.get_by_label("Cancelar").click();
    settle(&mut h);
    assert!(h.state().service.active_document().unwrap().is_untitled());
    assert_eq!(h.state().service.documents.len(), 2);
    assert_eq!(text(&h), draft);
    assert!(!path.exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("first.ts")).unwrap(),
        "keep"
    );
    key(&mut h, Modifiers::CTRL, Key::S);
    h.get_by_role_and_label(Role::TextInput, "Caminho").focus();
    h.step();
    key(&mut h, Modifiers::CTRL, Key::A);
    h.event(egui::Event::Text(path.to_string_lossy().into()));
    h.step();
    h.get_by_label("Salvar arquivo").click();
    settle(&mut h);
    assert_eq!(h.state().service.active_document().unwrap().path, path);
    assert!(!h.state().service.active_document().unwrap().is_untitled());
    assert_eq!(fs::read_to_string(&path).unwrap(), draft);
    assert_eq!(text(&h), draft);
    assert!(h.query_by_label("Markdown Source").is_some());
    let until = Instant::now() + Duration::from_secs(10);
    while !h
        .state()
        .service
        .entries
        .iter()
        .any(|entry| entry.path == path)
    {
        assert!(
            Instant::now() < until,
            "saved Markdown did not appear in the tree"
        );
        h.step();
        thread::sleep(Duration::from_millis(2));
    }
    h.state_mut()
        .service
        .dispatch(Command::CreateFile(path.clone()))
        .unwrap();
    settle(&mut h);
    assert!(h.state().service.last_error.is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), draft);
    assert_eq!(
        fs::read_to_string(dir.path().join("first.ts")).unwrap(),
        "keep"
    );
    assert_eq!(h.state().service.documents.len(), 2);
}
