//! Widget-level M1 acceptance with real temporary files and the real I/O worker.
//! These tests synthesize egui input; they do not claim OS/compositor input coverage.
use codisseu_core::{AppService, Command};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn harness(session: &Path) -> Harness<'static, CodisseuApp> {
    Harness::builder()
        .with_size([1280.0, 800.0])
        .with_max_steps(20)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(AppService::new(session.into())),
        )
}

fn settle(harness: &mut Harness<'_, CodisseuApp>) {
    // Always process the queued widget event, then wait on the actual worker state.
    harness.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.state().service.is_busy() || harness.state().folder_picker_pending() {
        assert!(
            Instant::now() < deadline,
            "I/O did not settle: {}",
            harness.state().service.status
        );
        thread::sleep(Duration::from_millis(1));
        harness.step();
    }
    harness.run(); // Includes the harness's accessibility/name checks.
}

fn enter_path(harness: &mut Harness<'_, CodisseuApp>, path: &Path) {
    harness
        .get_by_role_and_label(Role::TextInput, "Caminho")
        .click();
    settle(harness);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.event(egui::Event::Text(path.to_str().unwrap().into()));
    settle(harness);
    harness.get_by_label("Abrir caminho").click();
    settle(harness);
    assert!(
        harness.state().service.last_error.is_none(),
        "{:?}",
        harness.state().service.last_error
    );
}

fn choose_workspace(harness: &mut Harness<'_, CodisseuApp>, path: &Path) {
    // egui's harness cannot drive an OS window; native selection has a separate corpus.
    let path = path.to_owned();
    harness
        .state_mut()
        .set_folder_picker(move |_| Some(path.clone()))
        .unwrap();
    harness.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
    settle(harness);
}

fn replace_editor(harness: &mut Harness<'_, CodisseuApp>, text: &str) {
    harness.get_by_role(Role::MultilineTextInput).click();
    settle(harness);
    assert!(harness.get_by_role(Role::MultilineTextInput).is_focused());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.event(egui::Event::Text(text.into()));
    settle(harness);
    assert_eq!(
        harness.state().service.active_document().unwrap().text(),
        text
    );
}

#[test]
fn project_tree_tabs_keyboard_menu_save_and_session_restore() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    fs::create_dir_all(workspace.join("src")).unwrap();
    let php = workspace.join("src/index.php");
    let ts = workspace.join("src/app.ts");
    fs::write(&php, "<?php echo 'inicial';\n").unwrap();
    fs::write(&ts, "export const nome = 'inicial';\n").unwrap();
    let session = dir.path().join("state/session.json");
    let mut ui = harness(&session);
    choose_workspace(&mut ui, &workspace);
    assert_eq!(
        ui.state().service.workspace.as_deref(),
        Some(workspace.as_path())
    );
    ui.get_by_label("src").click();
    settle(&mut ui);
    ui.get_by_label("index.php").click();
    settle(&mut ui);
    // Drag the vertical splitter found in the actual accessibility tree.
    let splitter = ui
        .get_all_by_role(Role::Splitter)
        .filter(|node| node.rect().height() > node.rect().width())
        .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
        .expect("project panel must expose a resize handle")
        .rect()
        .center();
    let editor_left = ui.get_by_role(Role::MultilineTextInput).rect().left();
    ui.hover_at(splitter);
    ui.drag_at(splitter);
    ui.step();
    let destination = splitter + egui::vec2(60.0, 0.0);
    ui.hover_at(destination);
    ui.step();
    ui.drop_at(destination);
    settle(&mut ui);
    assert!(ui.get_by_role(Role::MultilineTextInput).rect().left() > editor_left + 30.0);
    let changed_php = "<?php echo 'Olá, café e Ω';\n";
    replace_editor(&mut ui, changed_php);
    assert!(ui.state().service.active_document().unwrap().is_dirty());

    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "<?php echo 'inicial';\n"
    );
    if ui.query_by_label("Editar").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Editar").click();
    settle(&mut ui);
    ui.get_by_label("Refazer          Ctrl+Shift+Z").click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        changed_php
    );

    if ui.query_by_label("Arquivo").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }

    ui.get_by_label("Arquivo").click();
    settle(&mut ui);
    ui.get_by_label("Salvar           Ctrl+S").click();
    settle(&mut ui);
    assert_eq!(fs::read_to_string(&php).unwrap(), changed_php);
    assert!(!ui.state().service.active_document().unwrap().is_dirty());

    ui.key_press_modifiers(Modifiers::COMMAND, Key::O);
    settle(&mut ui);
    enter_path(&mut ui, &ts);
    assert_eq!(ui.state().service.documents.len(), 2);
    let changed_ts = "export const nome = 'ação e cafe\u{301}';\n";
    replace_editor(&mut ui, changed_ts);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut ui);
    assert_eq!(fs::read_to_string(&ts).unwrap(), changed_ts);
    assert!(!ui.state().service.active_document().unwrap().is_dirty());

    // Exercise the actual Exit menu path, which waits for asynchronous session persistence.
    if ui.query_by_label("Arquivo").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }
    ui.get_by_label("Arquivo").click();
    settle(&mut ui);
    ui.get_by_label("Sair").click();
    settle(&mut ui);
    assert!(session.is_file(), "normal exit must persist a session");
    drop(ui);

    let mut restored = harness(&session);
    restored
        .state_mut()
        .service
        .dispatch(Command::RestoreSession)
        .unwrap();
    settle(&mut restored);
    assert!(restored.state().service.last_error.is_none());
    assert_eq!(
        restored.state().service.workspace.as_deref(),
        Some(workspace.as_path())
    );
    assert_eq!(restored.state().service.documents.len(), 2);
    assert_eq!(restored.state().service.active_document().unwrap().path, ts);
    assert_eq!(
        restored.state().service.active_document().unwrap().text(),
        changed_ts
    );
}

#[test]
fn save_conflict_is_visible_and_cancel_close_keeps_unsaved_text() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("conflict.php");
    fs::write(&file, "<?php echo 1;\n").unwrap();
    let mut ui = harness(&dir.path().join("state.json"));
    // This flow tests explicit Save and dirty-close cancellation.
    ui.state_mut()
        .service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    ui.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    settle(&mut ui);
    enter_path(&mut ui, &file);
    let unsaved = "<?php echo 'edição local';\n";
    replace_editor(&mut ui, unsaved);
    fs::write(&file, "<?php echo 'edição externa';\n").unwrap();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::S);
    settle(&mut ui);
    assert!(ui.state().service.last_error.is_some());
    let error = ui
        .state()
        .service
        .last_error
        .as_ref()
        .unwrap()
        .replace(['\r', '\n'], " ");
    assert!(
        ui.query_by_label(&error).is_some(),
        "Save failure must be visible in the status bar"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "<?php echo 'edição externa';\n"
    );
    assert!(ui.state().service.active_document().unwrap().is_dirty());
    assert!(ui.query_by_label("O arquivo mudou no disco").is_some());
    ui.get_by_label("Cancelar").click();
    settle(&mut ui);

    ui.key_press_modifiers(Modifiers::COMMAND, Key::W);
    settle(&mut ui);
    assert!(ui.query_by_label("Alterações não salvas").is_some());
    ui.get_by_label("Voltar ao editor").click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        unsaved
    );
    assert_eq!(ui.state().service.documents.len(), 1);
    assert!(
        ui.get_by_role(Role::MultilineTextInput).is_focused(),
        "cancelling close must restore editor focus"
    );

    ui.key_press_modifiers(Modifiers::COMMAND, Key::W);
    settle(&mut ui);
    ui.get_by_label("Descartar e fechar").click();
    settle(&mut ui);
    assert!(ui.state().service.documents.is_empty());
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "<?php echo 'edição externa';\n"
    );
}

#[test]
fn workspace_shortcut_and_corrupt_session_exit_preserve_original_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    fs::create_dir(&workspace).unwrap();
    let session = dir.path().join("session.json");
    let corrupt = b"{ deliberately invalid session";
    fs::write(&session, corrupt).unwrap();
    let mut ui = harness(&session);
    ui.state_mut()
        .service
        .dispatch(Command::RestoreSession)
        .unwrap();
    settle(&mut ui);
    assert!(ui.state().service.last_error.is_some());
    assert_eq!(fs::read(&session).unwrap(), corrupt);

    let error = ui
        .state()
        .service
        .last_error
        .as_ref()
        .unwrap()
        .replace(['\r', '\n'], " ");
    assert!(
        ui.query_by_label(&error).is_some(),
        "Restore failure must be visible in the status bar"
    );
    choose_workspace(&mut ui, &workspace);
    assert_eq!(
        ui.state().service.workspace.as_deref(),
        Some(workspace.as_path())
    );

    if ui.query_by_label("Arquivo").is_none() {
        ui.get_by_label("Menu principal").click();
        ui.run();
    }

    ui.get_by_label("Arquivo").click();
    settle(&mut ui);
    ui.get_by_label("Sair").click();
    settle(&mut ui);
    assert!(
        ui.query_by_label("Não foi possível salvar a sessão")
            .is_some()
    );
    assert_eq!(fs::read(&session).unwrap(), corrupt);
    ui.get_by_label("Sair sem salvar sessão").click();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        ui.step();
        if ui.output().viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "API cleanup was not acknowledged"
        );
        thread::sleep(Duration::from_millis(1));
    }
    // Closing now waits for the API worker even when the corrupt session is preserved.
    assert!(!ui.state().service.api.is_busy());
    assert_eq!(fs::read(&session).unwrap(), corrupt);
}
