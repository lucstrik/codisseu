use codisseu_core::{AppService, Command, ProviderState};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

#[track_caller]
fn wait(ui: &mut Harness<'_, CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        ui.step();
        if check(ui.state()) {
            ui.step();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "last={:?}, cursor={:?}, signature_error={:?}, signature_pending={}, completion_open={}, statuses={:?}",
            ui.state().service.last_error,
            ui.state().cursor,
            ui.state().service.signature_help.error,
            ui.state().service.signature_help.is_pending(),
            ui.state().service.completion.is_open(),
            ui.state().service.language.statuses,
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn signature_menu_error_escape_focus_and_no_shortcut_in_path_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("value.ts");
    fs::write(&path, "greet(1);").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    // These assistance tests explicitly verify unsaved buffers and manual Save.
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, app: &mut CodisseuApp| app.show(ui),
        CodisseuApp::new(service),
    );
    wait(&mut ui, |app| !app.service.is_busy());
    if ui.query_by_label("Exibir").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Exibir").click();
    ui.run();
    ui.get_by_label("Informações de parâmetros    Ctrl+P")
        .click();
    ui.run();
    assert!(ui.query_by_label("Assinatura da chamada").is_some());
    assert!(ui.query_all_by_label_contains("Inicie o provider").count() > 0);
    ui.key_press(Key::Escape);
    ui.run();
    assert!(ui.query_by_label("Assinatura da chamada").is_none());
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::O);
    ui.run();
    ui.key_press_modifiers(Modifiers::CTRL, Key::P);
    ui.run();
    assert!(ui.query_by_label("Assinatura da chamada").is_none());
    assert_eq!(fs::read_to_string(path).unwrap(), "greet(1);");
}

#[test]
#[ignore = "real PHP/TS providers; requires CODISSEU_PROVIDER_CONFIG"]
fn real_signature_keyboard_retrigger_unicode_unsaved_and_escape() {
    let config = std::env::var_os("CODISSEU_PROVIDER_CONFIG").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let source = "function greet(name: string, count: number): string { return name; }\n/* 🙂 e\u{301} */ greet(\"Storm\", 2);\n";
    let path = root.join("signature.ts");
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(root.join("session.json"));
    // These assistance tests explicitly verify unsaved buffers and manual Save.
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, app: &mut CodisseuApp| app.show(ui),
        CodisseuApp::new(service),
    );
    wait(&mut ui, |app| !app.service.is_busy());
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(path.clone()))
        .unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::LoadProviderConfig(config.into()))
        .unwrap();
    wait(&mut ui, |app| !app.service.is_busy());
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |app| {
        app.service.language.statuses.len() == 2
            && app
                .service
                .language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && !app.service.is_busy()
    });
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::End);
    ui.step();
    ui.key_press(Key::ArrowUp);
    ui.step();
    ui.key_press(Key::End);
    ui.step();
    for _ in 0..2 {
        ui.key_press(Key::ArrowLeft);
        ui.step();
    }
    ui.key_press_modifiers(Modifiers::CTRL, Key::P);
    wait(&mut ui, |app| app.service.signature_help.result.is_some());
    assert_eq!(
        ui.state()
            .service
            .signature_help
            .result
            .as_ref()
            .unwrap()
            .active_parameter,
        Some(1)
    );
    assert!(
        ui.query_by_label_contains("Parâmetro ativo: count: number")
            .is_some()
    );
    for _ in 0..5 {
        ui.key_press(Key::ArrowLeft);
        ui.step();
    }
    wait(&mut ui, |app| {
        app.service
            .signature_help
            .result
            .as_ref()
            .is_some_and(|r| r.active_parameter == Some(0))
    });
    assert!(
        ui.query_by_label_contains("Parâmetro ativo: name: string")
            .is_some()
    );
    ui.event(egui::Event::Text("X".into()));
    ui.step();
    wait(&mut ui, |app| {
        app.service
            .signature_help
            .result
            .as_ref()
            .is_some_and(|r| r.version == app.service.active_document().unwrap().version)
    });
    assert!(ui.state().service.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    ui.key_press(Key::Escape);
    ui.run();
    assert!(ui.query_by_label("Assinatura da chamada").is_none());
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    ui.step();
    assert_eq!(ui.state().service.active_document().unwrap().text(), source);
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    wait(&mut ui, |app| !app.service.is_busy());
}
