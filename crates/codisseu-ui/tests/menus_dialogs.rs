//! Menus and modal paths exercise the same asynchronous file commands as the editor.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let until = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
}

#[test]
fn menu_switches_by_hover_and_keeps_unavailable_actions_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let mut ui = Harness::builder()
        .with_size([800., 500.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(AppService::new(dir.path().join("session.json"))));
    if ui.query_by_label("Arquivo").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Arquivo").click();
    settle(&mut ui);
    let save = ui.get_by_label("Salvar           Ctrl+S");
    assert!(save.accesskit_node().is_disabled());
    assert_eq!(save.rect().height(), 24.);
    let edit = ui.get_by_label("Editar").rect().center();
    ui.hover_at(edit);
    settle(&mut ui);
    assert!(ui.query_by_label("Salvar           Ctrl+S").is_none());
    assert!(
        ui.get_by_label("Desfazer         Ctrl+Z")
            .accesskit_node()
            .is_disabled()
    );
    ui.key_press(Key::Escape);
    settle(&mut ui);
    assert!(ui.query_by_label("Desfazer         Ctrl+Z").is_none());
}

#[test]
fn modal_blocks_background_and_cancel_enter_restores_editor_focus() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.php");
    let b = dir.path().join("b.ts");
    fs::write(&a, "<?php echo 'ação';\n").unwrap();
    fs::write(&b, "export const a = 1;\n").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    for path in [&a, &b] {
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
    }
    let mut ui = Harness::builder()
        .with_size([800., 500.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    let a_tab = ui.get_by_label("a.php").rect().center();
    ui.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    settle(&mut ui);
    let input = ui.get_by_role_and_label(Role::TextInput, "Caminho");
    assert!(input.is_focused());
    assert_eq!(input.rect().height(), 28.);
    assert_eq!(ui.get_by_label("Cancelar").rect().height(), 28.);
    assert!(ui.get_by_label("Cancelar").rect().width() >= 72.);
    // A valid alternate file makes accidental submission on Cancel observable.
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(a.to_str().unwrap().into()));
    settle(&mut ui);
    ui.hover_at(a_tab);
    for pressed in [true, false] {
        ui.event(egui::Event::PointerButton {
            pos: a_tab,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        ui.step();
    }
    settle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().path, b);
    assert!(
        ui.query_by_role_and_label(Role::TextInput, "Caminho")
            .is_some()
    );
    for _ in 0..10 {
        if ui.get_by_label("Cancelar").is_focused() {
            break;
        }
        ui.key_press(Key::Tab);
        settle(&mut ui);
    }
    assert!(ui.get_by_label("Cancelar").is_focused());
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert!(
        ui.query_by_role_and_label(Role::TextInput, "Caminho")
            .is_none()
    );
    assert_eq!(ui.state().service.active_document().unwrap().path, b);
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    assert_eq!(fs::read_to_string(a).unwrap(), "<?php echo 'ação';\n");
}

#[test]
fn path_field_enter_opens_real_file_and_escape_preserves_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Olá.ts");
    fs::write(&path, "export const chuva = '🌧';\n").unwrap();
    let mut ui = Harness::builder()
        .with_size([900., 600.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(AppService::new(dir.path().join("session.json"))));
    ui.key_press_modifiers(Modifiers::COMMAND, Key::O);
    settle(&mut ui);
    ui.event(egui::Event::Text(path.to_str().unwrap().into()));
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().path, path);
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::O);
    settle(&mut ui);
    ui.key_press(Key::Escape);
    settle(&mut ui);
    assert!(
        ui.query_by_role_and_label(Role::TextInput, "Caminho")
            .is_none()
    );
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    assert!(!ui.state().service.active_document().unwrap().is_dirty());
}

#[test]
fn asynchronous_open_moves_focus_to_the_completed_document() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.php");
    let next = dir.path().join("next.ts");
    let first_text = "<?php echo 'preservado';\n";
    let next_text = "export const chuva = '🌧';\n";
    fs::write(&first, first_text).unwrap();
    fs::write(&next, next_text).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(first.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());

    // The shared application command completes through the real worker and poll.
    // There is no synchronous widget focus request or artificial timing delay:
    // focus must follow completion regardless of how quickly the worker reads.
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(next.clone()))
        .unwrap();
    settle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().path, next);
    assert!(
        ui.get_by_role(Role::MultilineTextInput).is_focused(),
        "the newly completed document must receive focus from the previous editor"
    );
    // TextEdit implements Ctrl+End as document-end on Linux. COMMAND alone
    // sets no ctrl bit in RawInput and only moved to the end of the current row.
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    ui.event(egui::Event::Text("// edição sem novo clique\n".into()));
    settle(&mut ui);
    let expected = format!("{next_text}// edição sem novo clique\n");
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        expected
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut ui);
    assert!(ui.state().service.last_error.is_none());
    assert_eq!(fs::read_to_string(next).unwrap(), expected);
    assert_eq!(fs::read_to_string(first).unwrap(), first_text);
}
