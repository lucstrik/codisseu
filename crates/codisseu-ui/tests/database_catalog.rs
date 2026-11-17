//! Real worker-backed explorer hierarchy; no company database is contacted.
use codisseu_core::{
    AppService, Command,
    remote::{ConnectionConfig, Engine, RemoteCommand},
};
use codisseu_ui::CodisseuApp;
use egui::{Key, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(15);
    while ui.state().service.is_busy() {
        assert!(
            Instant::now() < deadline,
            "{}",
            ui.state().service.remote_db.status
        );
        thread::sleep(Duration::from_millis(2));
        ui.step();
    }
    ui.run();
}
#[test]
#[ignore = "task-owned disposable MySQL; CODISSEU_SQL_WORKFLOW_MY_PORT"]
fn namespace_explorer_loads_only_expanded_tables_and_columns_and_opens_saved_profile() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.remote_db.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let config = ConnectionConfig {
        id: "ui-mysql-catalog".into(),
        name: "MySQL catalog fixture".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: std::env::var("CODISSEU_SQL_WORKFLOW_MY_PORT")
            .expect("disposable MySQL port")
            .parse()
            .unwrap(),
        username: "root".into(),
        database: String::new(),
        tls: false,
        timeout_secs: 3,
        idle_secs: 60,
        row_limit: 2,
        ..Default::default()
    };
    service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        }))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 1000.])
        .with_max_steps(100)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    assert!(ui.query_by_label("Arquivo SQLite").is_none());
    assert!(
        ui.state().service.remote_db.namespaces_loaded,
        "connect should fetch only namespace names once"
    );
    assert!(ui.state().service.remote_db.namespace_objects.is_empty());
    assert!(ui.state().service.remote_db.table_columns.is_empty());
    ui.get_by_role_and_label(Role::TreeItem, "corrier").click();
    settle(&mut ui);
    assert!(
        ui.state()
            .service
            .remote_db
            .namespace_objects
            .contains_key("corrier")
    );
    assert!(
        !ui.state()
            .service
            .remote_db
            .namespace_objects
            .contains_key("corrier_fat")
    );
    assert!(ui.state().service.remote_db.table_columns.is_empty());
    ui.get_by_role_and_label(Role::TreeItem, "remetentes")
        .click();
    settle(&mut ui);
    ui.key_press(Key::ArrowRight);
    settle(&mut ui);
    assert!(
        ui.state()
            .service
            .remote_db
            .table_columns
            .contains_key(&("corrier".into(), "remetentes".into()))
    );
    ui.get_by_role_and_label(Role::TreeItem, "nome");
    ui.get_by_role_and_label(Role::TreeItem, "id");
    ui.get_by_role_and_label(Role::TreeItem, "corrier_fat")
        .click();
    settle(&mut ui);
    ui.get_by_role_and_label(Role::TreeItem, "fatcte");
    assert!(
        !ui.state()
            .service
            .remote_db
            .table_columns
            .contains_key(&("corrier_fat".into(), "fatcte".into()))
    );
    // The toolbar uses semantic icon buttons with the same accessible action names.
    ui.get_by_label("Console").click();
    settle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(egui::Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    ui.get_by_role_and_label(Role::Button, "Execute");
    ui.get_by_role_and_label(Role::Button, "Stop");
    ui.get_by_role_and_label(Role::Button, "SQL Preview");
    ui.get_by_role_and_label(Role::Button, "Commit");
    ui.get_by_label("Data Sources and Drivers").click();
    settle(&mut ui);
    let name = ui.get_by_role_and_label(Role::TextInput, "Name:");
    assert_eq!(name.value().as_deref(), Some("MySQL catalog fixture"));
    assert!(ui.query_all_by_label("Adicionar datasource").count() > 0);
    ui.get_by_role_and_label(Role::Button, "Remover cadastro selecionado");
    ui.get_by_role_and_label(Role::Button, "Fechar datasources")
        .click();
    settle(&mut ui);
    ui.get_by_label("Desconectar datasource").click();
    settle(&mut ui);
    assert!(ui.state().service.remote_db.active.is_none());
    assert!(ui.state().service.remote_db.namespaces.is_empty());
}
