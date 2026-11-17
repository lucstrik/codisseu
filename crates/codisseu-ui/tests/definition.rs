//! Navigation through native widgets and real configured language servers.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
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
        assert!(
            ui.state().service.last_error.is_none(),
            "{:?}",
            ui.state().service.last_error
        );
        if condition(ui.state()) {
            ui.step();
            return;
        }
        assert!(Instant::now() < deadline, "{}", ui.state().service.status);
        thread::sleep(Duration::from_millis(5));
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
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    idle(ui);
}
fn find(ui: &mut Harness<'_, CodisseuApp>, text: &str) {
    ui.key_press_modifiers(Modifiers::COMMAND, Key::F);
    ui.step();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    idle(ui);
    ui.key_press(Key::Escape);
    ui.step();
    ui.step();
    ui.key_press(Key::ArrowLeft);
    ui.step();
}

#[test]
fn definition_without_provider_reports_error_and_history_stays_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ts");
    fs::write(&path, "const value = 42;").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    app.dispatch(Command::OpenFile(path)).unwrap();
    let mut ui = Harness::builder()
        .with_size([900., 600.])
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(app));
    idle(&mut ui);
    if ui.query_by_label("Navegar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Navegar").click();
    ui.run();
    assert!(
        ui.get_by_label("Voltar    Ctrl+Alt+Left")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        ui.get_by_label("Avançar    Ctrl+Alt+Right")
            .accesskit_node()
            .is_disabled()
    );
    ui.get_by_label("Ir à definição    F12").click();
    ui.step();
    assert!(
        ui.state()
            .service
            .navigation
            .error
            .as_deref()
            .unwrap()
            .contains("Inicie o provider")
    );
    assert_eq!(ui.state().service.documents.len(), 1);
    assert!(!ui.state().service.navigation.is_pending());
}

#[test]
fn multiple_definitions_keyboard_choice_and_escape_use_real_transport() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("main.ts");
    let first = root.join("first.ts");
    let second = root.join("second.ts");
    for path in [&source, &first, &second] {
        fs::write(path, "export const target = 42;\n").unwrap();
    }
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
    ui.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/../codisseu-language/tests/fixtures/definition_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                "multi".into(),
                root.join("protocol.jsonl").to_str().unwrap().into(),
                first.to_str().unwrap().into(),
                second.to_str().unwrap().into(),
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
            .any(|p| p.state == ProviderState::Ready)
            && !a.service.is_busy()
    });
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.step();
    ui.key_press(Key::F12);
    wait(&mut ui, |a| a.service.navigation.choices.len() == 2);
    assert!(ui.query_by_label("Escolher definição").is_some());
    ui.key_press(Key::Escape);
    idle(&mut ui);
    assert_eq!(ui.state().service.active_document().unwrap().path, source);
    assert!(ui.state().service.navigation.choices.is_empty());
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press(Key::F12);
    wait(&mut ui, |a| a.service.navigation.choices.len() == 2);
    ui.key_press(Key::ArrowDown);
    ui.step();
    ui.key_press(Key::Enter);
    wait(&mut ui, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path == second)
            && !a.service.is_busy()
    });
    assert_eq!(ui.state().cursor, (1, 14));
    assert_eq!(ui.state().service.documents.len(), 2);
    assert!(!ui.state().service.documents.iter().any(|d| d.path == first));
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}

#[test]
#[ignore = "real PHP+TS GUI navigation; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn php_ts_menu_shortcut_return_and_forward_keep_dirty_text_and_unicode_caret() {
    let config =
        std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("explicit provider config required");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("composer.json"),
        r#"{"name":"storm/ui-navigation","autoload":{"psr-4":{"StormFixture\\":"src/"}}}"#,
    )
    .unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022"},"include":["*.ts"]}"#,
    )
    .unwrap();
    let cases = [
        (
            "main.php",
            "src/Rain.php",
            "<?php\nuse StormFixture\\Rain;\nnew Rain();\n",
            "<?php\nnamespace StormFixture;\nclass Rain {}\n",
            "<?php\nuse StormFixture\\Rain;\necho '🌧 e\u{301}'; new Rain();\n// unsaved source",
            "<?php\nnamespace StormFixture;\n// unsaved target\nclass Rain {}\n",
            "Rain()",
            (4, 1),
        ),
        (
            "main.ts",
            "rain.ts",
            "import { rain } from './rain';\nrain();\n",
            "export function rain(){ return 0; }\n",
            "import { rain } from './rain';\nconsole.log('🌧 e\u{301}'); rain();\n// unsaved source",
            "/* 🌧 e\u{301} */ export function rain(){ return 42; }\n// unsaved target",
            "rain()",
            (1, 28),
        ),
    ];
    for (source, target, source_disk, target_disk, ..) in cases {
        fs::write(root.join(source), source_disk).unwrap();
        fs::write(root.join(target), target_disk).unwrap();
    }
    let composer = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .current_dir(&root)
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .output()
        .expect("Composer required");
    assert!(composer.status.success());
    let mut service = AppService::new(root.join("session.json"));
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
        .dispatch(Command::LoadProviderConfig(config.into()))
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
    for (source, target, source_disk, target_disk, source_text, target_text, call, target_caret) in
        cases
    {
        open(&mut ui, &root.join(target));
        edit(&mut ui, target_text);
        open(&mut ui, &root.join(source));
        edit(&mut ui, source_text);
        find(&mut ui, call);
        let source_caret = ui.state().cursor;
        assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
        if source.ends_with("php") {
            if ui.query_by_label("Navegar").is_none() {
                ui.get_by_label("Menu principal").click();
                ui.run();
            }
            ui.get_by_label("Navegar").click();
            ui.run();
            ui.get_by_label("Ir à definição    F12").click();
        } else {
            ui.key_press(Key::F12);
        }
        wait(&mut ui, |a| {
            a.service
                .active_document()
                .is_some_and(|d| d.path == root.join(target))
                && !a.service.is_busy()
        });
        assert_eq!(ui.state().cursor, target_caret);
        assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            target_text
        );
        if ui.query_by_label("Navegar").is_none() {
            ui.get_by_label("Menu principal").click();
            ui.run();
        }
        ui.get_by_label("Navegar").click();
        ui.run();
        ui.get_by_label("Voltar    Ctrl+Alt+Left").click();
        idle(&mut ui);
        assert_eq!(
            ui.state().service.active_document().unwrap().path,
            root.join(source)
        );
        assert_eq!(ui.state().cursor, source_caret);
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            source_text
        );
        ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::ArrowRight);
        idle(&mut ui);
        assert_eq!(
            ui.state().service.active_document().unwrap().path,
            root.join(target)
        );
        assert_eq!(ui.state().cursor, target_caret);
        assert_eq!(fs::read_to_string(root.join(source)).unwrap(), source_disk);
        assert_eq!(fs::read_to_string(root.join(target)).unwrap(), target_disk);
    }
    assert!(ui.state().service.documents.iter().all(|d| d.is_dirty()));
    ui.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(&mut ui);
}
