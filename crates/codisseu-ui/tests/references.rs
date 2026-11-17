use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(ui: &mut Harness<'_, CodisseuApp>, condition: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        ui.step();
        if condition(ui.state()) {
            ui.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}; {:?}",
            ui.state().service.status,
            ui.state().service.references.error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn idle(ui: &mut Harness<'_, CodisseuApp>) {
    wait(ui, |a| !a.service.is_busy());
}
fn open(ui: &mut Harness<'_, CodisseuApp>, path: &Path) {
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(path.into()))
        .unwrap();
    idle(ui);
}
fn focus_editor(ui: &mut Harness<'_, CodisseuApp>) {
    let point = ui.get_by_role(Role::MultilineTextInput).rect().left_top() + egui::vec2(90., 10.);
    ui.hover_at(point);
    for pressed in [true, false] {
        ui.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    ui.step();
}
fn edit(ui: &mut Harness<'_, CodisseuApp>, text: &str) {
    focus_editor(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    idle(ui);
}
fn request(ui: &mut Harness<'_, CodisseuApp>) {
    focus_editor(ui);
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    ui.step();
    // One native key press can carry a printable text event in the same frame.
    // Harness::event otherwise runs one frame per event, so use RawInput here.
    let modifiers = Modifiers::ALT.plus(Modifiers::SHIFT);
    ui.input_mut().events.extend([
        egui::Event::Key {
            key: Key::Num7,
            physical_key: Some(Key::Num7),
            pressed: true,
            repeat: false,
            modifiers,
        },
        egui::Event::Text("&".into()),
        egui::Event::Key {
            key: Key::Num7,
            physical_key: Some(Key::Num7),
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]);
    wait(ui, |a| {
        !a.service.references.items.is_empty() && !a.service.references.is_pending()
    });
}

#[test]
fn references_keyboard_click_escape_preserve_dirty_buffers_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("source.ts");
    let target = root.join("target.ts");
    fs::write(&source, "😀source").unwrap();
    fs::write(&target, "target()").unwrap();
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    open(&mut ui, &target);
    edit(&mut ui, "target() // não salvo");
    let target_id = ui.state().service.active.unwrap();
    open(&mut ui, &source);
    edit(&mut ui, "😀source // e\u{301} não salvo");
    let source_id = ui.state().service.active.unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codisseu-language/tests/fixtures/references_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "immediate".into(),
                root.join("transport.jsonl").display().to_string(),
                target.display().to_string(),
                root.join("unused.ts").display().to_string(),
            ],
        }]))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });
    request(&mut ui);
    ui.check_accessibility();
    assert!(ui.get_by_role(Role::ListBoxOption).is_focused());
    ui.key_press(Key::ArrowDown);
    ui.key_press(Key::ArrowUp);
    ui.key_press(Key::Enter);
    wait(&mut ui, |a| {
        a.service.active == Some(target_id) && !a.service.references.is_pending()
    });
    assert_eq!(ui.state().cursor, (1, 1));
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    assert_eq!(ui.state().service.references.items.len(), 1);
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::ArrowLeft);
    idle(&mut ui);
    assert_eq!(ui.state().service.active, Some(source_id));
    request(&mut ui);
    ui.get_by_role(Role::ListBoxOption).click();
    ui.step();
    assert_eq!(
        ui.state().service.active,
        Some(source_id),
        "single click previews only"
    );
    ui.key_press(Key::Enter);
    wait(&mut ui, |a| {
        a.service.active == Some(target_id) && !a.service.references.is_pending()
    });
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    assert!(ui.state().service.documents.iter().all(|d| d.is_dirty()));
    assert_eq!(
        ui.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == target_id)
            .unwrap()
            .text(),
        "target() // não salvo"
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), "😀source");
    assert_eq!(fs::read_to_string(&target).unwrap(), "target()");
    request(&mut ui);
    ui.key_press(Key::Escape);
    idle(&mut ui);
    assert!(
        ui.state().service.references.is_open(),
        "Escape preserves completed Find list"
    );
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}

#[test]
fn references_menu_reports_missing_provider_and_search_keeps_its_focus() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.ts");
    fs::write(&source, "target").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(source)).unwrap();
    let mut ui = Harness::builder().with_size([900., 600.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    if ui.query_by_label("Navegar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Navegar").click();
    ui.run();
    ui.get_by_label("Buscar referências    Shift+F12").click();
    ui.step();
    assert!(
        ui.state()
            .service
            .references
            .error
            .as_deref()
            .unwrap()
            .contains("Inicie o provider")
    );
    focus_editor(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::F);
    ui.step();
    ui.key_press_modifiers(Modifiers::ALT.plus(Modifiers::SHIFT), Key::Num7);
    ui.step();
    assert!(
        ui.get_by_role_and_label(Role::TextInput, "Localizar")
            .is_focused()
    );
    assert!(!ui.state().service.references.is_pending());
}

#[test]
fn collapsed_files_are_skipped_by_keyboard_and_long_lines_keep_scroll_extent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("source.ts");
    let first = root.join("a.ts");
    let second = root.join("b.ts");
    fs::write(&source, "source").unwrap();
    fs::write(&first, format!("target // {}", "long ".repeat(100))).unwrap();
    fs::write(&second, "target").unwrap();
    let mut service = AppService::new(root.join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut ui = Harness::builder().with_size([900., 600.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    open(&mut ui, &source);
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codisseu-language/tests/fixtures/references_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "two".into(),
                root.join("transport.jsonl").display().to_string(),
                first.display().to_string(),
                second.display().to_string(),
            ],
        }]))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });
    request(&mut ui);
    assert_eq!(ui.state().service.references.items.len(), 2);
    let width = ui
        .query_all_by_role(Role::ListBoxOption)
        .next()
        .unwrap()
        .rect()
        .width();
    assert!(
        width > 900.,
        "long result must allocate horizontal scroll extent: {width}"
    );
    ui.get_by_label("Recolher arquivo a.ts").click();
    ui.step();
    assert_eq!(ui.query_all_by_role(Role::ListBoxOption).count(), 1);
    let row = ui.get_by_role(Role::ListBoxOption).rect();
    let point = egui::pos2(160., row.center().y);
    ui.hover_at(point);
    for pressed in [true, false] {
        ui.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    ui.step();
    ui.key_press(Key::ArrowUp);
    ui.step();
    assert!(ui.get_by_role(Role::ListBoxOption).is_focused());
    ui.key_press(Key::Enter);
    wait(&mut ui, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path == second)
            && !a.service.references.is_pending()
    });
    assert_eq!(ui.state().cursor, (1, 1));
    assert_eq!(fs::read_to_string(&second).unwrap(), "target");
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}

#[test]
fn batched_history_then_save_preserves_physical_command_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ts");
    fs::write(&path, "base").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut ui = Harness::builder().with_size([900., 600.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    edit(&mut ui, "changed");
    for (key, expected) in [(Key::Z, "base"), (Key::Y, "changed")] {
        let modifiers = Modifiers::COMMAND;
        for key in [key, Key::S] {
            for pressed in [true, false] {
                ui.input_mut().events.push(egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed,
                    repeat: false,
                    modifiers,
                });
            }
        }
        idle(&mut ui);
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            expected
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert!(!ui.state().service.active_document().unwrap().is_dirty());
    }
    focus_editor(&mut ui);
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    ui.step();
    ui.input_mut().events.push(egui::Event::Text("x".into()));
    ui.input_mut().events.push(egui::Event::Key {
        key: Key::S,
        physical_key: Some(Key::S),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    idle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "changedx"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "changedx",
        "text input must reach the buffer before Save"
    );
    for key in [Key::S, Key::Z] {
        ui.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
    }
    idle(&mut ui);
    assert_eq!(fs::read_to_string(&path).unwrap(), "changedx");
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "changed"
    );
    assert!(
        ui.state().service.active_document().unwrap().is_dirty(),
        "Save then Undo must leave later undo unsaved"
    );
}
