use codisseu_core::{AppService, Command, ProviderState};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

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
            "{:?}",
            ui.state().service.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn documentation_menu_missing_provider_escape_and_edit_invalidation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("value.ts");
    fs::write(&path, "const value = 42;").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    // These assistance tests explicitly verify unsaved buffers and manual Save.
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut ui = Harness::builder().with_size([900., 600.]).build_ui_state(
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
    ui.get_by_label("Documentação do símbolo    Ctrl+Q").click();
    ui.step();
    ui.step();
    assert!(ui.query_by_label("LSP · documento atual").is_some());
    assert!(
        ui.state()
            .service
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("Inicie o provider"))
    );
    ui.key_press(Key::Escape);
    ui.step();
    ui.step();
    assert!(ui.query_by_label("LSP · documento atual").is_none());
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Q);
    ui.step();
    ui.step();
    assert!(ui.query_by_label("LSP · documento atual").is_some());
    let doc = ui.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    ui.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: "const changed = 2;".into(),
        })
        .unwrap();
    ui.step();
    ui.step();
    assert!(ui.query_by_label("LSP · documento atual").is_none());
}

#[test]
#[ignore = "real PHP/TS providers; requires CODISSEU_PROVIDER_CONFIG"]
fn real_documentation_at_unicode_caret_and_unsaved_edit() {
    let config = std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("explicit providers required");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("value.ts");
    let original = "/* 😀 e\u{301} */ const answer = 42;";
    fs::write(&path, original).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"include":["*.ts"]}"#,
    )
    .unwrap();
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
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Home);
    ui.step();
    let caret = original[..original.find("answer").unwrap()].chars().count() + 2;
    for _ in 0..caret {
        ui.key_press(Key::ArrowRight);
        ui.step();
    }
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Q);
    wait(&mut ui, |app| {
        app.service.language.hover.is_some() && !app.service.language.hover_pending()
    });
    assert!(
        ui.state()
            .service
            .language
            .hover
            .as_ref()
            .unwrap()
            .text
            .contains("answer")
    );
    assert!(ui.query_by_label_contains("answer: 42").is_some());
    ui.key_press(Key::Escape);
    ui.step();
    ui.step();
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("const answer = 'updated';".into()));
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Home);
    ui.step();
    for _ in 0..8 {
        ui.key_press(Key::ArrowRight);
        ui.step();
    }
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Q);
    wait(&mut ui, |app| {
        app.service
            .language
            .hover
            .as_ref()
            .is_some_and(|h| h.version == app.service.active_document().unwrap().version)
            && !app.service.language.hover_pending()
    });
    assert!(
        ui.state()
            .service
            .language
            .hover
            .as_ref()
            .unwrap()
            .text
            .contains("updated")
    );
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    ui.key_press(Key::Escape);
    ui.step();
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    wait(&mut ui, |app| !app.service.is_busy());
}
