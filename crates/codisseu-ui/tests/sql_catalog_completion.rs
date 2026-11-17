//! Real disposable MySQL catalog + egui keyboard input. Requires the explicitly
//! seeded corrier.remetentes(id,nome,ativo), corrier_fat.fatcte fixture; no writes.
use codisseu_core::{
    AppService, Command,
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand},
};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut idle = 0;
    while idle < 3 {
        ui.step();
        assert!(
            Instant::now() < deadline,
            "{} / {:?}",
            ui.state().service.remote_db.status,
            ui.state().service.remote_db.error
        );
        if ui.state().service.is_busy() {
            idle = 0;
            thread::sleep(Duration::from_millis(2));
        } else {
            idle += 1;
        }
    }
    ui.run();
}
fn setup() -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let port = std::env::var("CODISSEU_TEST_MYSQL_PORT")
        .expect("Set CODISSEU_TEST_MYSQL_PORT to the disposable seeded MySQL fixture")
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let sql = directory.path().join("catalog.sql");
    std::fs::write(&sql, "SELECT 7 AS sentinel;").unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    service.dispatch(Command::OpenFile(sql)).unwrap();
    service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config: ConnectionConfig {
                id: "catalog-ui-fixture".into(),
                name: "Catalog UI fixture".into(),
                engine: Engine::Mysql,
                host: "127.0.0.1".into(),
                port,
                username: "root".into(),
                database: String::new(),
                tls: false,
                timeout_secs: 3,
                idle_secs: 120,
                read_only: true,
                ..Default::default()
            },
            password: "codisseu_fixture".into(),
            test: false,
        }))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 1000.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    assert!(
        ui.state().service.remote_db.active.is_some(),
        "{:?}",
        ui.state().service.remote_db.error
    );
    (directory, ui)
}
fn text(ui: &mut Harness<'_, CodisseuApp>, value: &str) {
    ui.key_press(Key::Escape);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(value.into()));
    settle(ui);
    ui.key_press(Key::Escape);
    settle(ui);
    assert_eq!(ui.state().service.active_document().unwrap().text(), value);
}
fn cursor_before(ui: &mut Harness<'_, CodisseuApp>, marker: &str) {
    let document = ui.state().service.active_document().unwrap().text();
    let count = document[..document.find(marker).unwrap()].chars().count();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Home);
    for _ in 0..count {
        ui.key_press(Key::ArrowRight);
    }
    settle(ui);
}
fn option(ui: &Harness<'_, CodisseuApp>, name: &str) -> bool {
    ui.query_by_role_and_label(Role::ListBoxOption, name)
        .is_some()
}
fn close(ui: &mut Harness<'_, CodisseuApp>) {
    ui.key_press(Key::Escape);
    settle(ui);
    ui.state_mut()
        .service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Disconnect))
        .unwrap();
    settle(ui);
    assert!(ui.state().service.remote_db.active.is_none());
}

#[test]
#[ignore = "requires disposable seeded MySQL; CODISSEU_TEST_MYSQL_PORT"]
fn mysql_catalog_completion_inserts_real_tables_columns_and_preserves_undo_result() {
    let (directory, mut ui) = setup();
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.remote_db.result.as_ref().unwrap().rows,
        vec![vec![Cell::Number("7".into())]]
    );
    let revision = ui.state().service.remote_db.revision;

    text(&mut ui, "SELECT * FROM corrier.rem;");
    cursor_before(&mut ui, ";");
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(
        option(&ui, "remetentes"),
        "real table suggestion missing: {:?}",
        ui.state().service.remote_db.error
    );
    assert!(
        ui.state()
            .service
            .remote_db
            .namespaces
            .iter()
            .any(|s| s == "corrier_fat")
    );
    assert!(
        ui.state()
            .service
            .remote_db
            .completion_tables
            .get(&("corrier".into(), "rem".into()))
            .unwrap()
            .iter()
            .any(|t| t.name == "remetentes")
    );
    assert_eq!(
        ui.state().service.remote_db.revision,
        revision,
        "metadata must not replace the result revision"
    );
    assert_eq!(
        ui.state().service.remote_db.result.as_ref().unwrap().rows,
        vec![vec![Cell::Number("7".into())]]
    );
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT * FROM corrier.`remetentes`;"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT * FROM corrier.rem;"
    );
    ui.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT * FROM corrier.`remetentes`;"
    );

    text(&mut ui, "SELECT r.no FROM corrier.remetentes AS r;");
    cursor_before(&mut ui, " FROM");
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(
        option(&ui, "nome"),
        "real column suggestion missing: {:?}",
        ui.state().service.remote_db.error
    );
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT r.`nome` FROM corrier.remetentes AS r;"
    );

    text(&mut ui, "SELECT * FROM corrier_fat.fat LIMIT 1;");
    cursor_before(&mut ui, " LIMIT");
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(
        option(&ui, "fatcte"),
        "second database must expose its own real table"
    );
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT * FROM corrier_fat.`fatcte` LIMIT 1;"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.remote_db.error.is_none(),
        "{:?}",
        ui.state().service.remote_db.error
    );
    assert_eq!(
        ui.state()
            .service
            .remote_db
            .result
            .as_ref()
            .unwrap()
            .rows
            .len(),
        1
    );

    text(
        &mut ui,
        "SELECT i FROM corrier.remetentes r JOIN corrier_fat.fatcte f ON r.id=f.id LIMIT 1;",
    );
    cursor_before(&mut ui, " FROM");
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(
        option(&ui, "r.id"),
        "ambiguous id must offer qualified r.id"
    );
    assert!(
        option(&ui, "f.id"),
        "ambiguous id must offer qualified f.id"
    );
    assert!(!option(&ui, "id"), "ambiguous bare id must not be offered");
    ui.get_by_role_and_label(Role::ListBoxOption, "r.id")
        .click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT `r`.`id` FROM corrier.remetentes r JOIN corrier_fat.fatcte f ON r.id=f.id LIMIT 1;"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.remote_db.error.is_none(),
        "{:?}",
        ui.state().service.remote_db.error
    );
    assert_eq!(
        ui.state()
            .service
            .remote_db
            .result
            .as_ref()
            .unwrap()
            .columns,
        vec!["id"]
    );
    assert_eq!(
        ui.state()
            .service
            .remote_db
            .result
            .as_ref()
            .unwrap()
            .rows
            .len(),
        1
    );

    text(&mut ui, "SEL;");
    cursor_before(&mut ui, ";");
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(option(&ui, "SELECT"));
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT;"
    );

    for (sql, marker) in [("SELECT 'rem';", "';"), ("SELECT 1 -- rem\n;", "\n;")] {
        text(&mut ui, sql);
        cursor_before(&mut ui, marker);
        ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
        settle(&mut ui);
        assert!(
            ui.query_by_role(Role::ListBoxOption).is_none(),
            "completion appeared inside a literal/comment"
        );
    }
    text(&mut ui, "SELECT * FROM corrier");
    ui.event(egui::Event::Text(".".into()));
    settle(&mut ui);
    assert!(
        option(&ui, "remetentes"),
        "typing dot must trigger catalog completion"
    );
    ui.key_press(Key::ArrowLeft);
    settle(&mut ui);
    assert!(
        ui.query_by_role(Role::ListBoxOption).is_none(),
        "caret navigation must dismiss the old popup"
    );
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(option(&ui, "corrier"));
    let another = directory.path().join("another.sql");
    std::fs::write(&another, "SELECT 1;").unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::OpenFile(another))
        .unwrap();
    settle(&mut ui);
    assert!(
        ui.query_by_role(Role::ListBoxOption).is_none(),
        "document switch must dismiss the old popup"
    );

    text(&mut ui, "SELECT * FROM corrier.");
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    assert!(option(&ui, "remetentes"));
    let generation = ui.state().service.remote_db.session_generation;
    let config = ui
        .state()
        .service
        .remote_db
        .active
        .as_ref()
        .unwrap()
        .clone();
    ui.state_mut()
        .service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config,
            password: "codisseu_fixture".into(),
            test: false,
        }))
        .unwrap();
    settle(&mut ui);
    assert!(ui.state().service.remote_db.session_generation > generation);
    assert!(
        ui.query_by_role(Role::ListBoxOption).is_none(),
        "same-config reconnect must invalidate session-bound suggestions"
    );
    close(&mut ui);
}

#[test]
#[ignore = "requires disposable seeded MySQL; CODISSEU_TEST_MYSQL_PORT"]
fn mysql_ctrl_enter_targets_second_statement_and_recovers_after_sql_error() {
    let (_directory, mut ui) = setup();
    text(&mut ui, "SELECT 111 AS first;\n\nSELECT\n  222 AS second;");
    cursor_before(&mut ui, "222");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.remote_db.error.is_none(),
        "{:?}",
        ui.state().service.remote_db.error
    );
    assert_eq!(
        ui.state()
            .service
            .remote_db
            .result
            .as_ref()
            .unwrap()
            .columns,
        vec!["second"]
    );
    assert_eq!(
        ui.state().service.remote_db.result.as_ref().unwrap().rows,
        vec![vec![Cell::Number("222".into())]]
    );

    text(&mut ui, "SELECT CONNECTION_ID() AS connection;");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    let connection = ui
        .state()
        .service
        .remote_db
        .result
        .as_ref()
        .unwrap()
        .rows
        .clone();
    text(&mut ui, "SELECT corrier.remetentes LIMIT 10;");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(ui.state().service.remote_db.error.is_some());
    assert!(
        ui.state().service.remote_db.active.is_some(),
        "ordinary SQL error disconnected a healthy session"
    );
    text(&mut ui, "SELECT CONNECTION_ID() AS connection;");
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.remote_db.error.is_none(),
        "{:?}",
        ui.state().service.remote_db.error
    );
    assert_eq!(
        ui.state().service.remote_db.result.as_ref().unwrap().rows,
        connection,
        "successful next query must use the same real MySQL session"
    );
    close(&mut ui);
}
