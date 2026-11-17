//! More than512 real tables, GUI paging/filtering and prefix completion.
use codisseu_core::{
    AppService, Command,
    remote::{ConnectionConfig, Engine, RemoteCommand},
};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    thread,
    time::{Duration, Instant},
};
fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let end = Instant::now() + Duration::from_secs(20);
    while ui.state().service.is_busy() {
        assert!(
            Instant::now() < end,
            "{}",
            ui.state().service.remote_db.status
        );
        thread::sleep(Duration::from_millis(2));
        ui.step();
    }
    ui.run();
    assert!(
        ui.state().service.remote_db.error.is_none(),
        "{:?}",
        ui.state().service.remote_db.error
    );
}
fn bounds(ui: &Harness<'_, CodisseuApp>) -> (String, String, usize) {
    let page = &ui.state().service.remote_db.namespace_objects["catalog_many"];
    (
        page.first().unwrap().name.clone(),
        page.last().unwrap().name.clone(),
        page.len(),
    )
}
#[test]
#[ignore = "owned620tableMySQL; CODISSEU_CATALOG_MY_PORT"]
fn large_catalog_pages_filter_and_completion_reach_last_table_without_replacing_results() {
    let dir = tempfile::tempdir().unwrap();
    let sql = dir.path().join("many.sql");
    std::fs::write(&sql, "SELECT * FROM catalog_many.t61;").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(sql)).unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config: ConnectionConfig {
                id: "pagination-ui".into(),
                name: "Catalog620 fixture".into(),
                engine: Engine::Mysql,
                host: "127.0.0.1".into(),
                port: std::env::var("CODISSEU_CATALOG_MY_PORT")
                    .unwrap()
                    .parse()
                    .unwrap(),
                username: "root".into(),
                database: String::new(),
                tls: false,
                save_credentials: false,
                timeout_secs: 10,
                idle_secs: 300,
                ..Default::default()
            },
            password: "codisseu_fixture".into(),
            test: false,
        }))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 1000.])
        .with_max_steps(100)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    ui.state_mut()
        .service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Execute(
            "SELECT 7 AS preserved".into(),
        )))
        .unwrap();
    settle(&mut ui);
    let revision = ui.state().service.remote_db.revision;
    ui.get_by_role_and_label(Role::TreeItem, "catalog_many")
        .click();
    settle(&mut ui);
    assert_eq!(bounds(&ui), ("t000".into(), "t255".into(), 256));
    let editor_width = ui.get_by_role(Role::MultilineTextInput).rect().width();
    ui.get_by_label("Próximas tabelas de catalog_many").click();
    settle(&mut ui);
    assert_eq!(bounds(&ui), ("t256".into(), "t511".into(), 256));
    ui.get_by_label("Tabelas anteriores de catalog_many")
        .click();
    settle(&mut ui);
    assert_eq!(bounds(&ui), ("t000".into(), "t255".into(), 256));
    for _ in 0..2 {
        ui.get_by_label("Próximas tabelas de catalog_many").click();
        settle(&mut ui);
    }
    assert_eq!(bounds(&ui), ("t512".into(), "t619".into(), 108));
    ui.get_by_label("Primeira página de catalog_many").click();
    settle(&mut ui);
    assert_eq!(bounds(&ui), ("t000".into(), "t255".into(), 256));
    ui.get_by_role_and_label(Role::TextInput, "Filtrar tabelas de catalog_many")
        .click();
    settle(&mut ui);
    ui.event(egui::Event::Text("t61".into()));
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert_eq!(bounds(&ui), ("t610".into(), "t619".into(), 10));
    ui.get_by_role_and_label(Role::TextInput, "Filtrar tabelas de catalog_many")
        .click();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.key_press(Key::Backspace);
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert_eq!(bounds(&ui), ("t000".into(), "t255".into(), 256));
    ui.get_by_role(Role::MultilineTextInput).click();
    ui.key_press_modifiers(Modifiers::COMMAND, Key::End);
    ui.key_press(Key::ArrowLeft);
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::CTRL, Key::Space);
    settle(&mut ui);
    ui.get_by_role_and_label(Role::ListBoxOption, "t619")
        .scroll_to_me();
    settle(&mut ui);
    ui.step();
    settle(&mut ui);
    ui.get_by_role_and_label(Role::ListBoxOption, "t619")
        .click();
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT * FROM catalog_many.`t619`;"
    );
    assert_eq!(
        bounds(&ui),
        ("t000".into(), "t255".into(), 256),
        "completion must not replace explorer page"
    );
    assert!(
        (ui.get_by_role(Role::MultilineTextInput).rect().width() - editor_width).abs() < 2.,
        "filter must not expand the panel each frame"
    );
    assert_eq!(ui.state().service.remote_db.revision, revision);
    assert_eq!(
        ui.state()
            .service
            .remote_db
            .result
            .as_ref()
            .unwrap()
            .columns,
        ["preserved"]
    );
    for i in 0..10 {
        ui.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::CompleteTables {
                namespace: "catalog_many".into(),
                prefix: format!("t{i}"),
            }))
            .unwrap();
        settle(&mut ui);
        assert!(ui.state().service.remote_db.completion_tables.len() <= 8);
    }
    ui.state_mut()
        .service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Disconnect))
        .unwrap();
    settle(&mut ui);
    assert!(ui.state().service.remote_db.namespace_pages.is_empty());
    assert!(ui.state().service.remote_db.completion_tables.is_empty());
    assert!(ui.state().service.remote_db.completion_more.is_empty());
}
