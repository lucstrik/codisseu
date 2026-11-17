//! M3 document search through widgets, real workers and real files.
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
    let deadline = Instant::now() + Duration::from_secs(10);
    for _ in 0..3 {
        ui.step();
        while ui.state().service.is_busy() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
            ui.step();
        }
        ui.run();
    }
}

fn field(ui: &mut Harness<'_, CodisseuApp>, label: &str, value: &str) {
    ui.get_by_role_and_label(Role::TextInput, label).click();
    settle(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    if value.is_empty() {
        ui.key_press(Key::Backspace);
    } else {
        ui.event(egui::Event::Text(value.into()));
    }
    settle(ui);
}

#[test]
fn unicode_find_navigate_replace_undo_and_save_use_real_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unicode.ts");
    let original = "// 😀 ação e\u{301}\r\nconst x = 'ação';\r\n// ação";
    fs::write(&path, original).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(40)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::H);
    settle(&mut ui);
    assert!(
        ui.get_by_role_and_label(Role::TextInput, "Localizar")
            .is_focused()
    );
    let find = ui
        .get_by_role_and_label(Role::TextInput, "Localizar")
        .rect();
    assert!((find.center().y - ui.get_by_label("Fechar busca").rect().center().y).abs() < 1.);
    let replacement = ui
        .get_by_role_and_label(Role::TextInput, "Substituir por")
        .rect();
    assert!(
        (replacement.center().y - ui.get_by_label("Substituir todas").rect().center().y).abs() < 1.
    );
    field(&mut ui, "Localizar", "ação");
    assert!(ui.query_by_label("1 de 3").is_some());
    ui.key_press(Key::F3);
    settle(&mut ui);
    assert!(ui.query_by_label("2 de 3").is_some());
    field(&mut ui, "Substituir por", "chuva 🌧");
    ui.get_by_label("Substituir ocorrência").click();
    settle(&mut ui);
    let one = "// 😀 ação e\u{301}\r\nconst x = 'chuva 🌧';\r\n// ação";
    assert_eq!(ui.state().service.active_document().unwrap().text(), one);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        original,
        "replace must not save implicitly"
    );
    ui.get_by_label("Substituir todas").click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        one.replace("ação", "chuva 🌧")
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        one,
        "replace-all is one undo step"
    );
    ui.key_press(Key::Escape);
    settle(&mut ui);
    assert!(
        ui.query_by_role_and_label(Role::TextInput, "Localizar")
            .is_none()
    );
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut ui);
    assert_eq!(fs::read_to_string(&path).unwrap(), one);
    assert!(ui.state().service.last_error.is_none());
}

#[test]
fn find_menu_empty_missing_and_changed_document_are_safe_in_narrow_window() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.php");
    let b = dir.path().join("b.ts");
    fs::write(&a, "<?php // target target\n").unwrap();
    fs::write(&b, "// elsewhere\n").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(a.clone())).unwrap();
    let mut ui = Harness::builder()
        .with_size([900., 600.])
        .with_max_steps(40)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    if ui.query_by_label("Editar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Editar").click();
    settle(&mut ui);
    ui.get_by_label("Substituir       Ctrl+H").click();
    settle(&mut ui);
    assert!(
        ui.get_by_label("Substituir todas")
            .accesskit_node()
            .is_disabled()
    );
    field(&mut ui, "Localizar", "target");
    assert!(ui.query_by_label("1 de 2").is_some());
    ui.key_press_modifiers(Modifiers::SHIFT, Key::F3);
    settle(&mut ui);
    assert!(ui.query_by_label("2 de 2").is_some());
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(b.clone()))
        .unwrap();
    settle(&mut ui);
    assert!(ui.query_by_label("0 de 0").is_some());
    assert!(
        ui.get_by_label("Substituir todas")
            .accesskit_node()
            .is_disabled()
    );
    field(&mut ui, "Localizar", "elsewhere");
    assert!(ui.query_by_label("1 de 1").is_some());
    field(&mut ui, "Localizar", "");
    assert!(
        ui.get_by_label("Substituir ocorrência")
            .accesskit_node()
            .is_disabled()
    );
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "// elsewhere\n"
    );
    assert_eq!(fs::read_to_string(&a).unwrap(), "<?php // target target\n");
    for label in ["Próximo", "Anterior", "Fechar busca", "Substituir todas"] {
        let rect = ui.get_by_label(label).rect();
        assert!(
            rect.left() >= 0. && rect.right() <= 900. && rect.bottom() <= 600.,
            "{label}: {rect:?}"
        );
    }
}

#[test]
fn find_selection_reaches_real_editor_and_search_undo_does_not_touch_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.ts");
    let original = format!(
        "// 😀 target\r\n{}// e\u{301} target",
        "// linha\r\n".repeat(80)
    );
    fs::write(&path, &original).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(40)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    let doc = ui.state().service.active_document().unwrap();
    let id = doc.id;
    let version = doc.version;
    let edited = format!("{original}!");
    ui.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: edited.clone(),
        })
        .unwrap();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::F);
    settle(&mut ui);
    field(&mut ui, "Localizar", "target");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().text(), edited);
    field(&mut ui, "Localizar", "target");
    ui.key_press(Key::F3);
    settle(&mut ui);
    assert!(ui.query_by_label("2 de 2").is_some());
    ui.key_press(Key::Escape);
    settle(&mut ui);
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.event(egui::Event::Text("🌧".into()));
    settle(&mut ui);
    let expected = format!(
        "// 😀 target\r\n{}// e\u{301} 🌧!",
        "// linha\r\n".repeat(80)
    );
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        expected
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().text(), edited);
}
