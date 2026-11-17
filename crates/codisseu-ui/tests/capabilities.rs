//! Real SQLite exercised through egui widgets and the shared document editor.
use codisseu_core::AppService;
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(
            Instant::now() < deadline,
            "Timed out: {}",
            ui.state().service.status
        );
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
}
fn text_field(ui: &mut Harness<'_, CodisseuApp>, label: &str, text: &str) {
    ui.get_by_role_and_label(Role::TextInput, label).click();
    settle(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    settle(ui);
}
fn path_dialog(ui: &mut Harness<'_, CodisseuApp>, path: &Path) {
    text_field(ui, "Caminho", path.to_str().unwrap());
    ui.get_by_label("Abrir caminho").click();
    settle(ui);
    assert!(
        ui.state().service.last_error.is_none(),
        "{:?}",
        ui.state().service.last_error
    );
}
fn editor_text(ui: &mut Harness<'_, CodisseuApp>, text: &str) {
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(ui);
    assert!(ui.get_by_role(Role::MultilineTextInput).is_focused());
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    settle(ui);
    assert_eq!(ui.state().service.active_document().unwrap().text(), text);
}
#[test]
fn connect_schema_select_and_invalid_sql_through_widgets() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("fixture.sqlite");
    let sql = temp.path().join("query.sql");
    {
        let db = Connection::open(&database).unwrap();
        db.execute_batch("CREATE TABLE people(id INTEGER PRIMARY KEY,name TEXT,note TEXT); INSERT INTO people VALUES(1,'Olá',NULL),(2,'Codisseu','');").unwrap();
    }
    fs::write(&sql, "SELECT id, name, note FROM people ORDER BY id;\n").unwrap();
    let app = CodisseuApp::new(AppService::new(temp.path().join("session.json")));
    let mut ui = Harness::builder()
        .with_size([1440.0, 900.0])
        .with_max_steps(20)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    assert!(!ui.state().service.db.connected);
    ui.get_by_label("Adicionar datasource").click();
    settle(&mut ui);
    ui.get_by_label("SQLite (arquivo)").click();
    settle(&mut ui);
    text_field(&mut ui, "Arquivo SQLite", database.to_str().unwrap());
    ui.get_by_label("Conectar SQLite").click();
    settle(&mut ui);
    assert!(
        ui.state().service.db.last_error.is_none(),
        "{:?}",
        ui.state().service.db.last_error
    );
    assert!(ui.state().service.db.connected);
    assert!(ui.query_by_label("people").is_some());
    assert_eq!(ui.state().service.db.schema[0].columns.len(), 3);
    ui.get_by_label("Abrir SQL…").click();
    settle(&mut ui);
    path_dialog(&mut ui, &sql);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    let result = ui.state().service.db.result.as_ref().unwrap();
    assert_eq!(result.rows.len(), 2);
    assert_eq!(
        result.rows[0],
        vec![Value::Integer(1), Value::Text("Olá".into()), Value::Null]
    );
    assert_eq!(result.rows[1][2], Value::Text("".into()));
    assert!(ui.query_by_label("Olá").is_some());
    assert!(ui.query_by_label("NULL").is_some());
    editor_text(&mut ui, "SELECT 'ação; café' AS greeting;\n");
    ui.get_by_label("Executar SQL").click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows[0][0],
        Value::Text("ação; café".into())
    );
    editor_text(&mut ui, "SELECT does_not_exist FROM people;\n");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(ui.state().service.db.last_error.is_some());
    assert!(
        ui.state().service.db.result.is_none(),
        "failed query must not leave old result presented as current"
    );
    assert!(
        ui.query_all_by_label_contains("does_not_exist")
            .next()
            .is_some()
    );
    ui.get_by_label("Desconectar").click();
    settle(&mut ui);
    assert!(!ui.state().service.db.connected);
    let db = Connection::open(&database).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM people", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}
