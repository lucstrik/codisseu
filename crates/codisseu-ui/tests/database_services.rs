//! Services reflects actual sessions/results; profile existence is not a connection.
use codisseu_core::{AppService, Command, DbCommand, remote::ConnectionConfig};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(Instant::now() < deadline, "own local worker did not settle");
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
}

fn execute(ui: &mut Harness<'_, CodisseuApp>) {
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(ui);
}

#[test]
fn saved_profile_alone_does_not_appear_as_a_live_service() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("query.sql");
    fs::write(&path, "SELECT 1;").unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(40)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    // Explicit synthetic profile: this test never authenticates a remote session.
    ui.state_mut()
        .service
        .remote_db
        .profiles
        .push(ConnectionConfig {
            id: "unconnected-own-profile".into(),
            name: "Saved but disconnected".into(),
            ..ConnectionConfig::default()
        });
    execute(&mut ui);
    assert!(ui.state().service.remote_db.active.is_none());
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Serviços de banco")
            .is_some()
    );
    assert!(ui.query_by_label("Nenhuma conexão ativa").is_some());
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Serviço Saved but disconnected")
            .is_none()
    );
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Console query.sql")
            .is_none()
    );
}

#[test]
fn actual_sqlite_result_console_activates_without_reexecuting_and_retains_closed_origin() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("fixture.sqlite");
    rusqlite::Connection::open(&database).unwrap();
    let path = directory.path().join("query.sql");
    fs::write(&path, "SELECT 'Ada' AS name;").unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    service
        .dispatch(Command::Database(DbCommand::Connect(database)))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    execute(&mut ui);
    assert!(ui.state().service.db.last_error.is_none());
    let document = ui.state().service.active.unwrap();
    let revision = ui.state().service.db.result_revision();
    assert!(revision.is_some());
    assert_eq!(
        ui.get_by_role_and_label(Role::TreeItem, "Console query.sql")
            .rect()
            .height(),
        24.
    );
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Serviço fixture.sqlite")
            .is_some()
    );
    assert!(
        ui.query_by_label("Ada").is_some(),
        "row text appears once in the result, never in the tree"
    );
    ui.get_by_label("Mostrar saída do serviço").click();
    settle(&mut ui);
    assert!(ui.query_by_label("Ada").is_none());
    ui.get_by_label("Mostrar resultado do serviço").click();
    settle(&mut ui);
    assert!(ui.query_by_label("Ada").is_some());
    assert_eq!(ui.state().service.db.result_revision(), revision);

    ui.get_by_role_and_label(Role::TreeItem, "Serviços de banco")
        .click();
    settle(&mut ui);
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Console query.sql")
            .is_none()
    );
    ui.key_press(Key::ArrowRight);
    settle(&mut ui);
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Console query.sql")
            .is_some()
    );

    let other = directory.path().join("other.sql");
    fs::write(&other, "SELECT * FROM missing_own_test_table;").unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(other))
        .unwrap();
    settle(&mut ui);
    assert_ne!(ui.state().service.active, Some(document));
    ui.get_by_role_and_label(Role::TreeItem, "Console query.sql")
        .click();
    settle(&mut ui);
    assert_eq!(ui.state().service.active, Some(document));
    assert_eq!(ui.state().service.db.result_revision(), revision);
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows[0][0],
        rusqlite::types::Value::Text("Ada".into())
    );

    ui.state_mut()
        .service
        .dispatch(Command::Close {
            id: document,
            discard: true,
        })
        .unwrap();
    settle(&mut ui);
    let current = ui.state().service.active;
    ui.get_by_role_and_label(Role::TreeItem, "Console query.sql")
        .click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active,
        current,
        "closed source is informational and is not reopened"
    );
    assert_eq!(ui.state().service.db.result_revision(), revision);
    // SQLite deliberately clears its last result as soon as a query is sent.
    // A failed query therefore leaves no result origin, including no false
    // attribution to other.sql. This differs from the remote SQL worker.
    execute(&mut ui);
    assert!(ui.state().service.db.last_error.is_some());
    assert!(ui.state().service.db.connected);
    assert_eq!(ui.state().service.db.result_revision(), None);
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Console query.sql")
            .is_none()
    );
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "Console other.sql")
            .is_none()
    );
}
