//! Real widget input, with controlled transport for failures and real providers separately.
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
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        ui.step();
        if condition(ui.state()) {
            ui.step();
            return;
        }
        assert!(
            Instant::now() < until,
            "{}; {:?}",
            ui.state().service.status,
            ui.state().service.completion.error
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
fn edit(ui: &mut Harness<'_, CodisseuApp>, text: &str) {
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
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    idle(ui);
}
fn end(ui: &mut Harness<'_, CodisseuApp>) {
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
    ui.key_press_modifiers(Modifiers::CTRL, Key::End);
    let text = ui.state().service.active_document().unwrap().text();
    let expected = (
        text.chars().filter(|c| *c == '\n').count() + 1,
        text.rsplit('\n').next().unwrap().chars().count() + 1,
    );
    wait(ui, |a| a.cursor == expected);
}
fn request(ui: &mut Harness<'_, CodisseuApp>) {
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    wait(ui, |a| !a.service.completion.items.is_empty());
}
#[test]
fn controlled_completion_accept_escape_focus_undo_and_text_input_are_real() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("source.ts");
    fs::write(&path, "😀foo").unwrap();
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
    let mut ui = Harness::builder().with_size([900., 600.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    open(&mut ui, &path);
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codisseu-language/tests/fixtures/completion_server.py");
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                script.display().to_string(),
                "immediate".into(),
                root.join("transport.jsonl").display().to_string(),
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
    end(&mut ui);
    request(&mut ui);
    ui.check_accessibility();
    assert!(
        ui.query_by_role_and_label(Role::ListBoxOption, "food")
            .is_some()
    );
    ui.key_press(Key::Escape);
    idle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "😀foo"
    );
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    request(&mut ui);
    ui.key_press(Key::Tab);
    idle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "😀food"
    );
    assert_eq!(ui.state().cursor, (1, 6));
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "😀foo"
    );
    end(&mut ui);
    request(&mut ui);
    ui.get_by_role_and_label(Role::ListBoxOption, "food")
        .click();
    idle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "😀food"
    );
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut ui);
    end(&mut ui);
    request(&mut ui);
    ui.key_press(Key::ArrowLeft);
    ui.step();
    assert!(!ui.state().service.completion.is_open());
    end(&mut ui);
    request(&mut ui);
    ui.event(egui::Event::Text("x".into()));
    idle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "😀foox"
    );
    assert!(!ui.state().service.completion.is_open());
    assert_eq!(fs::read_to_string(&path).unwrap(), "😀foo");
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}
#[test]
fn missing_provider_error_is_visible_and_search_does_not_request_completion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ts");
    fs::write(&path, "foo").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    // These assistance tests explicitly verify unsaved buffers and manual Save.
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut ui = Harness::builder().with_size([800., 600.]).build_ui_state(
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    if ui.query_by_label("Editar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Editar").click();
    ui.run();
    ui.get_by_label("Completar código    Ctrl+Space").click();
    ui.step();
    assert!(
        ui.state()
            .service
            .completion
            .error
            .as_deref()
            .unwrap()
            .contains("Inicie o provider")
    );
    assert_eq!(
        ui.query_all_by_label_contains("Inicie o provider").count(),
        1
    );
    ui.key_press(Key::Escape);
    ui.step();
    ui.get_by_label("Adicionar datasource").click();
    ui.run();
    ui.get_by_label("SQLite (arquivo)").click();
    ui.run();
    ui.get_by_role_and_label(Role::TextInput, "Arquivo SQLite")
        .click();
    ui.step();
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    ui.step();
    assert!(
        ui.get_by_role_and_label(Role::TextInput, "Arquivo SQLite")
            .is_focused()
    );
    assert!(!ui.state().service.completion.is_open());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::F);
    ui.step();
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    ui.step();
    assert!(!ui.state().service.completion.is_open());
}
#[test]
#[ignore = "real PHP/TS completion widgets; requires CODISSEU_PROVIDER_CONFIG"]
fn php_ts_unsaved_member_completion_menu_keyboard_undo_redo_save() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022"},"include":["*.ts"]}"#,
    )
    .unwrap();
    let cases = [
        (
            "weather.ts",
            "class Weather { rainfall(): string { return 'rain'; } raincoat(): string { return 'coat'; } }\nconst weather = new Weather();\nconsole.log('🌧 e\u{301}'); weather.rain",
        ),
        (
            "weather.php",
            "<?php\nclass Weather { public function rainfall(): string { return 'rain'; } public function raincoat(): string { return 'coat'; } }\n$weather = new Weather();\necho '🌧 e\u{301}'; $weather->rain",
        ),
    ];
    for (name, _) in cases {
        fs::write(
            root.join(name),
            if name.ends_with("php") {
                "<?php\n// disk\n"
            } else {
                "// disk\n"
            },
        )
        .unwrap();
    }
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
        |ui, a: &mut CodisseuApp| a.show(ui),
        CodisseuApp::new(service),
    );
    idle(&mut ui);
    ui.state_mut()
        .service
        .dispatch(Command::LoadProviderConfig(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("explicit providers required")
                .into(),
        ))
        .unwrap();
    idle(&mut ui);
    ui.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut ui, |a| {
        a.service.language.statuses.len() == 2
            && a.service
                .language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });
    for (name, text) in cases {
        let path = root.join(name);
        open(&mut ui, &path);
        edit(&mut ui, text);
        end(&mut ui);
        if name.ends_with("php") {
            if ui.query_by_label("Editar").is_none() {
                ui.get_by_label("Menu principal").click();
                ui.run();
            }
            ui.get_by_label("Editar").click();
            ui.run();
            ui.get_by_label("Completar código    Ctrl+Space").click();
            wait(&mut ui, |a| !a.service.completion.items.is_empty());
        } else {
            request(&mut ui);
        }
        let index = ui
            .state()
            .service
            .completion
            .items
            .iter()
            .position(|i| i.label == "rainfall")
            .expect("real member rainfall");
        for _ in 0..index {
            ui.key_press(Key::ArrowDown);
            ui.step();
        }
        ui.key_press(Key::Enter);
        idle(&mut ui);
        // PHP call-site completion now adds a pair and places the caret inside;
        // TypeScript keeps the provider's original plain-text edit.
        let php_call = name.ends_with("php");
        let expected = format!("{}fall{}", text, if php_call { "()" } else { "" });
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            expected
        );
        assert_eq!(
            ui.state().cursor.1,
            expected.lines().last().unwrap().chars().count() + usize::from(!php_call)
        );
        if php_call {
            wait(&mut ui, |app| !app.service.signature_help.is_pending());
            let signature = ui
                .state()
                .service
                .signature_help
                .result
                .as_ref()
                .expect("accepting the PHP method requests its real parameter information");
            assert!(!signature.signatures.is_empty());
            assert!(
                signature.signatures[0].parameters.is_empty(),
                "rainfall has no parameters"
            );
        }
        assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
        assert!(fs::read_to_string(&path).unwrap().contains("// disk"));
        ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        idle(&mut ui);
        assert_eq!(ui.state().service.active_document().unwrap().text(), text);
        ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        idle(&mut ui);
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            expected
        );
        ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
        idle(&mut ui);
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
    }
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}
