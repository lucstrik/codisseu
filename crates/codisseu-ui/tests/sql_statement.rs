//! Real SQLite + egui keyboard routing; no query response is mocked.
use codisseu_core::{AppService, Command, DbCommand};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use rusqlite::{Connection, types::Value};
use std::{
    thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(Instant::now() < deadline, "{}", ui.state().service.status);
        thread::sleep(Duration::from_millis(2));
        ui.step();
    }
    ui.run();
}

#[test]
fn ctrl_enter_runs_current_multiline_statement_and_exact_selection() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("fixture.sqlite");
    Connection::open(&database).unwrap().close().unwrap();
    let sql = directory.path().join("query.sql");
    let original = "SELECT 'first; 🦀';\n\nSELECT\n  22 AS second;\n\nSELECT 33; SELECT 44;\n";
    std::fs::write(&sql, original).unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    service.dispatch(Command::OpenFile(sql.clone())).unwrap();
    service
        .dispatch(Command::Database(DbCommand::Connect(database)))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 900.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Home);
    for _ in 0..3 {
        ui.key_press(Key::ArrowDown);
    }
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.db.last_error.is_none(),
        "{:?}",
        ui.state().service.db.last_error
    );
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows,
        vec![vec![Value::Integer(22)]]
    );
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().columns,
        vec!["second"]
    );

    // Select only SELECT 44 on a line with two statements. No script expansion.
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::End);
    ui.key_press(Key::ArrowUp);
    ui.key_press(Key::End);
    for _ in 0..10 {
        ui.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
    }
    settle(&mut ui);
    let selection = ui
        .state()
        .service
        .active_document()
        .unwrap()
        .selection()
        .range();
    let selected: String = original
        .chars()
        .skip(selection.start)
        .take(selection.len())
        .collect();
    assert_eq!(selected, "SELECT 44;");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.db.last_error.is_none(),
        "{:?}",
        ui.state().service.db.last_error
    );
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows,
        vec![vec![Value::Integer(44)]]
    );

    // Selecting just the numeric token must be rejected, never expanded to SQL.
    ui.key_press(Key::ArrowRight);
    ui.key_press(Key::ArrowLeft);
    for _ in 0..2 {
        ui.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
    }
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(ui.state().service.db.last_error.is_some());
    assert!(ui.state().service.db.result.is_none());
    assert_eq!(std::fs::read_to_string(sql).unwrap(), original);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        original
    );
}
