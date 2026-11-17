use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(ui: &mut Harness<'_, CodisseuApp>) {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        ui.step();
        if !ui.state().service.is_busy() {
            ui.step();
            return;
        }
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn separate_native_style_text_frames_group_undo_and_cursor_save_paste_split_history() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.ts");
    fs::write(&p, "// ").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(p.clone())).unwrap();
    let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    ui.step();
    for c in "weather".chars() {
        ui.event(egui::Event::Text(c.to_string()));
        ui.step();
    }
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// weather"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(ui.state().service.active_document().unwrap().text(), "// ");
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    ui.step();
    ui.key_press(Key::ArrowLeft);
    ui.step();
    ui.key_press(Key::ArrowRight);
    ui.step();
    ui.event(egui::Event::Text("x".into()));
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// weather"
    );
    // Insert before an identical suffix; the resulting caret must anchor the delta.
    ui.key_press_modifiers(Modifiers::CTRL, Key::Home);
    ui.step();
    for _ in 0..3 {
        ui.key_press(Key::ArrowRight);
        ui.step();
    }
    for c in "weather".chars() {
        ui.event(egui::Event::Text(c.to_string()));
        ui.step();
    }
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// weatherweather"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// weather"
    );
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    ui.step();
    ui.event(egui::Event::Paste("雨😀e\u{301}".into()));
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    idle(&mut ui);
    assert_eq!(fs::read_to_string(&p).unwrap(), "// weather雨😀e\u{301}");
    ui.event(egui::Event::Text("!".into()));
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// weather雨😀e\u{301}"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// weather"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(ui.state().service.active_document().unwrap().text(), "// ");
    assert_eq!(fs::read_to_string(&p).unwrap(), "// weather雨😀e\u{301}");
}

fn history_fixture(
    text: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    Harness<'static, CodisseuApp>,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.ts");
    fs::write(&path, text).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.step();
    (dir, path, ui)
}
const HISTORY_TEXT: &str = "// storm editor history\nconst weather = \"rain\";\n// 😀 é end\n";
fn select_weather(ui: &mut Harness<'_, CodisseuApp>, reverse: bool) -> codisseu_core::Selection {
    ui.key_press_modifiers(Modifiers::CTRL, Key::Home);
    ui.step();
    ui.key_press(Key::ArrowDown);
    ui.key_press(Key::Home);
    ui.step();
    for _ in 0..6 {
        ui.key_press(Key::ArrowRight);
        ui.step();
    }
    if reverse {
        for _ in 0..7 {
            ui.key_press(Key::ArrowRight);
            ui.step();
        }
        for _ in 0..7 {
            ui.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
            ui.step();
        }
    } else {
        for _ in 0..7 {
            ui.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
            ui.step();
        }
    }
    let start = HISTORY_TEXT[..HISTORY_TEXT.find("weather").unwrap()]
        .chars()
        .count();
    let selected = if reverse {
        codisseu_core::Selection {
            anchor: start + 7,
            head: start,
        }
    } else {
        codisseu_core::Selection {
            anchor: start,
            head: start + 7,
        }
    };
    assert_eq!(
        ui.state().service.active_document().unwrap().selection(),
        selected
    );
    selected
}
#[test]
fn undo_restores_directed_selection_before_the_next_frame_can_replace_it() {
    for reverse in [false, true] {
        let (_dir, path, mut ui) = history_fixture(HISTORY_TEXT);
        let selected = select_weather(&mut ui, reverse);
        for c in "codisseu".chars() {
            ui.event(egui::Event::Text(c.to_string()));
            ui.step();
        }
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            HISTORY_TEXT.replace("weather", "codisseu")
        );
        ui.key_press_modifiers(Modifiers::CTRL, Key::Home);
        ui.step();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        ui.step();
        assert_eq!(
            ui.state().service.active_document().unwrap().selection(),
            selected
        );
        // No neutral frame or reposition command between Undo and this input.
        ui.event(egui::Event::Text("X".into()));
        ui.step();
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            HISTORY_TEXT.replace("weather", "X")
        );
        assert_eq!(fs::read_to_string(path).unwrap(), HISTORY_TEXT);
    }
}
#[test]
fn redo_restores_final_caret_for_immediate_typing_and_menu_undo_preserves_selection() {
    let (_dir, _path, mut ui) = history_fixture(HISTORY_TEXT);
    let selected = select_weather(&mut ui, true);
    ui.event(egui::Event::Text("storm".into()));
    ui.step();
    if ui.query_by_label("Editar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Editar").click();
    ui.run();
    ui.get_by_label("Desfazer         Ctrl+Z").click();
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().selection(),
        selected
    );
    if ui.query_by_label("Editar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Editar").click();
    ui.run();
    ui.get_by_label("Refazer          Ctrl+Shift+Z").click();
    ui.step();
    ui.event(egui::Event::Text("X".into()));
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        HISTORY_TEXT.replace("weather", "stormX")
    );
}
#[test]
fn selection_is_document_owned_and_history_restores_unicode_paste_after_navigation() {
    let text = "😀e\u{301} tail\r\n";
    let (dir, path, mut ui) = history_fixture(text);
    ui.key_press_modifiers(Modifiers::CTRL, Key::Home);
    ui.step();
    // The custom editor now moves over extended graphemes: emoji + e/combining.
    for _ in 0..2 {
        ui.key_press_modifiers(Modifiers::SHIFT, Key::ArrowRight);
        ui.step();
    }
    let selected = ui.state().service.active_document().unwrap().selection();
    let id = ui.state().service.active.unwrap();
    let second = dir.path().join("other.ts");
    fs::write(&second, "other").unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(second))
        .unwrap();
    idle(&mut ui);
    ui.get_by_label("main.ts").click();
    ui.step();
    ui.step();
    assert_eq!(ui.state().service.active, Some(id));
    ui.event(egui::Event::Paste("雨".into()));
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "雨 tail\r\n"
    );
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(ui.state().service.active_document().unwrap().text(), text);
    assert_eq!(
        ui.state().service.active_document().unwrap().selection(),
        selected
    );
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    ui.step();
    ui.event(egui::Event::Text("X".into()));
    ui.step();
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "雨X tail\r\n"
    );
    assert_eq!(fs::read_to_string(path).unwrap(), text);
}
