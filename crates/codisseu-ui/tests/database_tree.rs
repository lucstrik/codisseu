//! SQLite catalog rows share the product tree component and expose real metadata.
use codisseu_core::AppService;
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use rusqlite::Connection;
use std::{
    thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(Instant::now() < deadline, "{}", ui.state().service.status);
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
}

#[test]
fn real_database_tree_supports_selection_keyboard_metadata_and_stable_ids() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catálogo.sqlite");
    let database = Connection::open(&path).unwrap();
    database
        .execute_batch(
            "CREATE TABLE people(id INTEGER PRIMARY KEY, apelido TEXT NOT NULL, observação TEXT);",
        )
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(AppService::new(temp.path().join("session.json"))));
    settle(&mut ui);
    ui.get_by_label("Adicionar datasource").click();
    settle(&mut ui);
    ui.get_by_label("SQLite (arquivo)").click();
    settle(&mut ui);
    assert_eq!(
        ui.get_by_role_and_label(Role::TextInput, "Arquivo SQLite")
            .rect()
            .height(),
        28.
    );
    assert_eq!(ui.get_by_label("Conectar SQLite").rect().height(), 28.);
    assert!(
        ui.get_by_label("Conectar SQLite")
            .accesskit_node()
            .is_disabled()
    );
    ui.get_by_role_and_label(Role::TextInput, "Arquivo SQLite")
        .click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(path.display().to_string()));
    settle(&mut ui);
    ui.get_by_label("Conectar SQLite").click();
    settle(&mut ui);
    assert!(ui.state().service.db.connected);
    assert!(ui.state().service.db.last_error.is_none());
    assert_eq!(ui.state().service.db.schema[0].columns.len(), 3);
    assert!(
        ui.query_by_role_and_label(Role::Tree, "Objetos de banco")
            .is_some()
    );

    let table_id = ui
        .get_by_role_and_label(Role::TreeItem, "people")
        .accesskit_node()
        .id();
    let column_id = ui
        .get_by_role_and_label(Role::TreeItem, "apelido")
        .accesskit_node()
        .id();
    for name in [
        "catálogo.sqlite",
        "main",
        "people",
        "id",
        "apelido",
        "observação",
    ] {
        let node = ui.get_by_role_and_label(Role::TreeItem, name);
        assert_eq!(node.rect().height(), 24., "row {name}");
        assert!(node.accesskit_node().data().level().is_some());
    }
    let column = ui.get_by_role_and_label(Role::TreeItem, "apelido");
    let description = column.accesskit_node().data().description().unwrap();
    assert!(description.contains("main.people.apelido"));
    assert!(description.contains("Tipo: TEXT"));
    assert!(description.contains("Chave primária: não"));
    assert!(description.contains("NOT NULL"));
    assert_eq!(column.accesskit_node().data().is_expanded(), None);
    let key = ui.get_by_role_and_label(Role::TreeItem, "id");
    assert!(
        key.accesskit_node()
            .data()
            .description()
            .unwrap()
            .contains("Chave primária: sim")
    );
    assert!(
        ui.get_by_role_and_label(Role::TreeItem, "observação")
            .accesskit_node()
            .data()
            .description()
            .unwrap()
            .contains("NULL permitido")
    );

    // Clicking a branch selects and collapses it; arrows expand and visit children.
    ui.get_by_role_and_label(Role::TreeItem, "people").click();
    settle(&mut ui);
    let table = ui.get_by_role_and_label(Role::TreeItem, "people");
    assert!(table.is_focused());
    assert_eq!(table.accesskit_node().data().is_selected(), Some(true));
    assert_eq!(table.accesskit_node().data().is_expanded(), Some(false));
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "apelido")
            .is_none()
    );
    ui.key_press(Key::ArrowRight);
    settle(&mut ui);
    assert_eq!(
        ui.get_by_role_and_label(Role::TreeItem, "people")
            .accesskit_node()
            .data()
            .is_expanded(),
        Some(true)
    );
    ui.key_press(Key::ArrowDown);
    settle(&mut ui);
    assert!(ui.get_by_role_and_label(Role::TreeItem, "id").is_focused());
    ui.key_press(Key::ArrowDown);
    settle(&mut ui);
    let selected = ui.get_by_role_and_label(Role::TreeItem, "apelido");
    assert!(selected.is_focused());
    assert_eq!(selected.accesskit_node().data().is_selected(), Some(true));
    assert_eq!(selected.accesskit_node().id(), column_id);
    ui.key_press(Key::ArrowLeft);
    settle(&mut ui);
    assert!(
        ui.get_by_role_and_label(Role::TreeItem, "people")
            .is_focused()
    );
    ui.key_press(Key::Enter);
    settle(&mut ui);
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "apelido")
            .is_none()
    );
    ui.key_press(Key::Space);
    settle(&mut ui);
    ui.get_by_role_and_label(Role::TreeItem, "apelido").click();
    settle(&mut ui);
    assert_eq!(
        ui.get_by_role_and_label(Role::TreeItem, "apelido")
            .accesskit_node()
            .data()
            .is_selected(),
        Some(true)
    );

    // Refresh changes actual schema order; the selected object keeps its identity.
    database
        .execute_batch("CREATE TABLE aaa(new_column BLOB);")
        .unwrap();
    ui.get_by_label("Atualizar schema").click();
    settle(&mut ui);
    assert_eq!(ui.state().service.db.schema.len(), 2);
    assert!(ui.query_by_role_and_label(Role::TreeItem, "aaa").is_some());
    assert_eq!(
        ui.get_by_role_and_label(Role::TreeItem, "people")
            .accesskit_node()
            .id(),
        table_id
    );
    let selected = ui.get_by_role_and_label(Role::TreeItem, "apelido");
    assert_eq!(selected.accesskit_node().id(), column_id);
    assert_eq!(selected.accesskit_node().data().is_selected(), Some(true));
    ui.check_accessibility();

    ui.get_by_label("Desconectar").click();
    settle(&mut ui);
    assert!(!ui.state().service.db.connected);
    assert!(
        ui.query_by_role_and_label(Role::TreeItem, "people")
            .is_none()
    );
    assert!(ui.query_by_label("Conectar SQLite").is_some());
}
