//! Explicit integration: real providers controlled through the native egui widgets.
use codisseu_core::{AppService, ProviderState};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn wait(ui: &mut Harness<'_, CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        ui.step();
        assert!(
            ui.state().service.last_error.is_none(),
            "{:?}",
            ui.state().service.last_error
        );
        assert!(
            ui.state().service.language.last_error.is_none(),
            "{:?}",
            ui.state().service.language.last_error
        );
        if check(ui.state()) {
            // Settle layout/accessibility even while providers request continuous repaint.
            ui.step();
            ui.check_accessibility();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Timed out: {:?}; {:?}",
            ui.state().service.language.statuses,
            ui.state().service.language.diagnostics
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn idle(ui: &mut Harness<'_, CodisseuApp>) {
    wait(ui, |app| {
        !app.service.is_busy() && !app.folder_picker_pending()
    });
}
fn text_field(ui: &mut Harness<'_, CodisseuApp>, label: &str, text: &str) {
    ui.get_by_role_and_label(Role::TextInput, label).click();
    idle(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    idle(ui);
}
fn path_dialog(ui: &mut Harness<'_, CodisseuApp>, path: &Path) {
    text_field(ui, "Caminho", path.to_str().unwrap());
    ui.get_by_label("Abrir caminho").click();
    idle(ui);
}
fn open_file(ui: &mut Harness<'_, CodisseuApp>, path: &Path) {
    ui.key_press_modifiers(Modifiers::COMMAND, Key::O);
    idle(ui);
    path_dialog(ui, path);
}

#[test]
#[ignore = "explicit real provider GUI integration; requires CODISSEU_PROVIDER_CONFIG"]
fn configure_start_jump_utf16_edit_clear_and_stop_via_widgets() {
    let config_path = PathBuf::from(
        std::env::var_os("CODISSEU_PROVIDER_CONFIG")
            .expect("set CODISSEU_PROVIDER_CONFIG; this test never silently skips"),
    );
    assert!(config_path.is_file());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let php = root.join("sample.php");
    let ts = root.join("ação.ts");
    let bad_ts = "export const emoji = '🌧'; export const count: number = 'wrong';\n";
    fs::write(&php, "<?php $broken = ;\n").unwrap();
    fs::write(&ts, bad_ts).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022","noEmit":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440.0, 1000.0])
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(AppService::new(root.join("session.json"))),
        );
    let selected_root = root.clone();
    ui.state_mut()
        .set_folder_picker(move |_| Some(selected_root.clone()))
        .unwrap();
    ui.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::O,
    );
    idle(&mut ui);
    open_file(&mut ui, &ts);
    open_file(&mut ui, &php);
    assert!(
        !ui.state().service.providers_running(),
        "opening files must not launch processes"
    );
    ui.get_by_label("Configurar providers").click();
    idle(&mut ui);
    text_field(
        &mut ui,
        "Arquivo de configuração",
        config_path.to_str().unwrap(),
    );
    ui.get_by_label("Carregar configuração").click();
    idle(&mut ui);
    assert_eq!(ui.state().service.provider_configs.len(), 2);
    ui.get_by_label("Iniciar providers").click();
    wait(&mut ui, |app| {
        app.service.language.statuses.len() == 2
            && app
                .service
                .language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && app
                .service
                .language
                .diagnostics
                .iter()
                .any(|d| d.path == ts && d.severity == 1)
            && app
                .service
                .language
                .diagnostics
                .iter()
                .any(|d| d.path == php && d.severity == 1)
    });
    ui.get_by_label("Close window").click();
    idle(&mut ui);
    ui.get_by_label("Mostrar Problemas").click();
    idle(&mut ui);
    assert!(
        ui.query_by_label("TypeScript: publicação sem versão")
            .is_some()
    );
    ui.get_by_label_contains("Type 'string' is not assignable to type 'number'.")
        .click();
    let target_byte = bad_ts.find("count").unwrap();
    let target_column = bad_ts[..target_byte].chars().count() + 1;
    wait(&mut ui, |app| {
        app.service.active_document().is_some_and(|d| d.path == ts)
            && app.cursor == (1, target_column)
    });
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.event(egui::Event::Text("checked_".into()));
    idle(&mut ui);
    let mut inserted = bad_ts.to_string();
    inserted.insert_str(target_byte, "checked_");
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        inserted,
        "clicking diagnostic places the caret at the UTF-16 range after emoji"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    let good_ts = "export const emoji = '🌧'; export const count: number = 42;\n";
    ui.event(egui::Event::Text(good_ts.into()));
    idle(&mut ui);
    let version = ui.state().service.active_document().unwrap().version;
    wait(&mut ui, |app| {
        app.service.language.analyzed_versions.get(&ts) == Some(&version)
            && !app
                .service
                .language
                .diagnostics
                .iter()
                .any(|d| d.path == ts && d.severity <= 2)
    });
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        good_ts
    );
    assert!(
        ui.query_by_label_contains("Type 'string' is not assignable to type 'number'.")
            .is_none()
    );
    assert!(ui.state().service.language.unversioned_paths.contains(&ts));
    assert_eq!(
        fs::read_to_string(&ts).unwrap(),
        bad_ts,
        "real provider analyzed unsaved editor text"
    );
    ui.get_by_label("Configurar providers").click();
    idle(&mut ui);
    ui.get_by_label("Parar providers").click();
    wait(&mut ui, |app| {
        !app.service.is_busy()
            && app.service.language.statuses.len() == 2
            && app
                .service
                .language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Stopped)
    });
    assert!(ui.state().service.language.diagnostics.is_empty());
    assert!(ui.state().service.language.unversioned_paths.is_empty());
}
