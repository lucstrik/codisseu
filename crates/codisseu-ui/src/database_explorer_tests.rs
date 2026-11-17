use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{}",
            h.state().service.remote_db.status
        );
        thread::sleep(Duration::from_millis(2));
    }
    h.run();
}
fn harness() -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    harness_scaled(1.)
}
fn harness_scaled(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.show_project = false;
    app.show_activity = false;
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_step_dt(1. / 120.)
        .with_max_steps(60)
        .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
    settle(&mut h);
    (dir, h)
}
fn profile(id: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: id.into(),
        name: format!("Fixture {id}"),
        engine: Engine::Memcached,
        save_credentials: false,
        host: "127.0.0.1".into(),
        tls: false,
        timeout_secs: 2,
        idle_secs: 60,
        ..Default::default()
    }
}
fn click_at(h: &mut Harness<'_, CodisseuApp>, pos: egui::Pos2, button: egui::PointerButton) {
    h.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    h.step();
}
// A counted protocol fixture, not a claim of real Memcached compatibility.
fn counted_server() -> (u16, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = Arc::new(AtomicUsize::new(0));
    let counted = count.clone();
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        counted.fetch_add(1, Ordering::SeqCst);
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "version\r\n");
        stream.write_all(b"VERSION 1.6.fixture\r\n").unwrap();
        line.clear();
        // Keep the same socket alive until the app is dropped.
        let _ = reader.read_line(&mut line);
    });
    (port, count, thread)
}

#[test]
fn context_menu_has_distinct_rows_and_properties_for_right_clicked_source() {
    let (_dir, mut h) = harness();
    let first = profile("one");
    let second = profile("two");
    h.state_mut().service.remote_db.profiles = vec![first.clone(), second.clone()];
    h.state_mut().database_ui.select(first);
    h.run();
    h.get_by_label(&second.name).click_secondary();
    h.run();
    assert_eq!(
        h.state().database_ui.selected_source.as_deref(),
        Some("two")
    );
    let connect = h.get_by_role_and_label(Role::Button, "Conectar").rect();
    let properties = h
        .get_by_role_and_label(Role::Button, "Propriedades…")
        .rect();
    assert!(
        connect.max.y <= properties.min.y,
        "{connect:?} / {properties:?}"
    );
    assert!(h.query_by_label("Propriedades / Conectar…").is_none());
    h.get_by_role_and_label(Role::Button, "Propriedades…")
        .click();
    h.run();
    assert!(h.state().database_ui.dialog);
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Name:")
            .value()
            .as_deref(),
        Some(second.name.as_str())
    );
}

#[test]
fn double_right_click_connects_through_open_context_popup_once() {
    let (port, count, server) = counted_server();
    let (_dir, mut h) = harness();
    let mut config = profile("right");
    config.port = port;
    h.state_mut().service.remote_db.profiles = vec![config.clone()];
    h.run();
    let pos = h.get_by_label(&config.name).rect().center();
    click_at(&mut h, pos, egui::PointerButton::Secondary);
    h.run();
    assert!(h.query_by_label("Propriedades…").is_some());
    // The popup is already present for this second pointer event.
    click_at(&mut h, pos, egui::PointerButton::Secondary);
    settle(&mut h);
    assert_eq!(
        h.state()
            .service
            .remote_db
            .active
            .as_ref()
            .map(|c| c.id.as_str()),
        Some("right")
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(!h.state().database_ui.dialog);
    assert!(h.query_by_label("Propriedades…").is_none());
    // Repeating on the active source preserves the session and does not reconnect.
    let generation = h.state().service.remote_db.session_generation;
    let pos = h.get_by_label(&config.name).rect().center();
    click_at(&mut h, pos, egui::PointerButton::Secondary);
    click_at(&mut h, pos, egui::PointerButton::Secondary);
    settle(&mut h);
    assert_eq!(h.state().service.remote_db.session_generation, generation);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    drop(h);
    server.join().unwrap();
}

#[test]
fn double_left_click_uses_same_worker_connection_action() {
    let (port, count, server) = counted_server();
    let (_dir, mut h) = harness();
    let mut config = profile("left");
    config.port = port;
    h.state_mut().service.remote_db.profiles = vec![config.clone()];
    h.run();
    let pos = h.get_by_label(&config.name).rect().center();
    click_at(&mut h, pos, egui::PointerButton::Primary);
    click_at(&mut h, pos, egui::PointerButton::Primary);
    settle(&mut h);
    assert_eq!(
        h.state()
            .service
            .remote_db
            .active
            .as_ref()
            .map(|c| c.id.as_str()),
        Some("left")
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(!h.state().database_ui.dialog);
    drop(h);
    server.join().unwrap();
}

#[test]
fn connect_gestures_preserve_transaction_and_local_edits() {
    for transaction in [false, true] {
        let (_dir, mut h) = harness();
        let config = profile("protected");
        h.state_mut().service.remote_db.profiles = vec![config.clone()];
        if transaction {
            h.state_mut().service.remote_db.transaction = true;
        } else {
            h.state_mut()
                .database_ui
                .staged
                .insert((0, 0), Cell::Text("pending".into()));
        }
        h.run();
        let pos = h.get_by_label(&config.name).rect().center();
        click_at(&mut h, pos, egui::PointerButton::Secondary);
        h.run();
        assert!(
            h.get_by_role_and_label(Role::Button, "Conectar")
                .accesskit_node()
                .is_disabled()
        );
        click_at(&mut h, pos, egui::PointerButton::Secondary);
        settle(&mut h);
        assert!(h.state().service.remote_db.active.is_none());
        assert!(h.state().service.remote_db.error.is_some());
        assert_eq!(h.state().service.remote_db.transaction, transaction);
        assert_eq!(
            h.state().database_ui.staged.len(),
            usize::from(!transaction)
        );
    }
}

#[test]
fn sidebar_visibility_preserves_loaded_catalog_expansion_and_zero_commands() {
    let (_dir, mut h) = harness();
    let mut config = profile("cached");
    config.engine = Engine::Mysql;
    let db = &mut h.state_mut().service.remote_db;
    db.profiles = vec![config.clone()];
    db.active = Some(config);
    db.namespaces = vec!["fixture".into()];
    db.namespaces_loaded = true;
    db.namespace_objects.insert(
        "fixture".into(),
        vec![remote::DbObject {
            namespace: "fixture".into(),
            name: "people".into(),
            kind: "TABLE".into(),
            columns: Vec::new(),
        }],
    );
    db.table_columns
        .insert(("fixture".into(), "people".into()), vec![]);
    h.run();
    h.state_mut()
        .database_ui
        .expanded_namespaces
        .insert("fixture".into());
    h.state_mut()
        .database_ui
        .expanded_tables
        .insert(("fixture".into(), "people".into()));
    h.run();
    for _ in 0..4 {
        h.get_by_label("Alternar painel Database").click();
        h.run();
        assert!(!h.state().show_database);
        h.get_by_label("Alternar painel Database").click();
        h.run();
        assert!(h.state().show_database);
        h.get_by_role_and_label(Role::TreeItem, "people");
        assert!(!h.state().service.remote_db.is_busy());
        assert!(h.state().service.remote_db.error.is_none());
        assert!(
            h.state()
                .database_ui
                .expanded_tables
                .contains(&("fixture".into(), "people".into()))
        );
        assert_eq!(
            h.state().service.remote_db.namespace_objects["fixture"].len(),
            1
        );
        assert!(
            h.state()
                .service
                .remote_db
                .table_columns
                .contains_key(&("fixture".into(), "people".into()))
        );
    }
    assert_eq!(h.state().database_ui.catalog_commands, 0);
}

#[test]
#[ignore = "task-owned MySQL fixture; CODISSEU_SQL_MY_PORT"]
fn real_mysql_sidebar_reopen_does_not_dispatch_catalog_queries() {
    let (_dir, mut h) = harness();
    let config = ConnectionConfig {
        id: "real-cached".into(),
        name: "MySQL explorer fixture".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: std::env::var("CODISSEU_SQL_MY_PORT")
            .unwrap()
            .parse()
            .unwrap(),
        database: "codisseu".into(),
        username: "root".into(),
        tls: false,
        idle_secs: 300,
        timeout_secs: 5,
        ..Default::default()
    };
    h.state_mut()
        .service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config,
            password: "codisseu_fixture".into(),
            test: false,
        }))
        .unwrap();
    settle(&mut h);
    assert!(
        h.state().service.remote_db.error.is_none(),
        "{:?}",
        h.state().service.remote_db.error
    );
    h.get_by_role_and_label(Role::TreeItem, "codisseu").click();
    settle(&mut h);
    let initial_commands = h.state().database_ui.catalog_commands;
    let initial_namespaces = h.state().service.remote_db.namespaces.clone();
    let initial_objects = h.state().service.remote_db.namespace_objects["codisseu"].len();
    let generation = h.state().service.remote_db.session_generation;
    for _ in 0..4 {
        h.get_by_label("Alternar painel Database").click();
        h.run();
        h.get_by_label("Alternar painel Database").click();
        settle(&mut h);
        assert_eq!(h.state().database_ui.catalog_commands, initial_commands);
        assert_eq!(h.state().service.remote_db.namespaces, initial_namespaces);
        assert_eq!(
            h.state().service.remote_db.namespace_objects["codisseu"].len(),
            initial_objects
        );
        assert_eq!(h.state().service.remote_db.session_generation, generation);
    }
}

#[test]
fn direct_connection_action_respects_busy_worker_without_replacing_session() {
    let (_dir, mut h) = harness();
    h.state_mut()
        .service
        .remote_db
        .dispatch(RemoteCommand::Inspect)
        .unwrap();
    // Do not poll: this exercises the service's accepted-command busy contract.
    assert!(h.state().service.remote_db.is_busy());
    h.state_mut().connect_datasource(profile("while-busy"));
    assert!(h.state().service.remote_db.is_busy());
    assert!(h.state().service.remote_db.active.is_none());
    assert_eq!(
        h.state().service.remote_db.error.as_deref(),
        Some("Aguarde ou cancele a operação atual")
    );
}

#[test]
fn rds_tls_form_supports_automatic_ca_and_explicit_override_without_connecting() {
    let (_dir, mut h) = harness();
    h.state_mut().database_ui.select(ConnectionConfig {
        id: "rds-fixture".into(),
        name: "AWS RDS QA".into(),
        host: "fixture.abc.us-east-1.rds.amazonaws.com".into(),
        save_credentials: false,
        ..Default::default()
    });
    h.state_mut().database_ui.dialog = true;
    h.run();
    h.get_by_label("SSL").click();
    h.run();
    assert!(h.state().database_ui.config.automatic_rds_ca());
    assert!(h.state().database_ui.config.tls);
    let field = h.get_by_role_and_label(Role::TextInput, "CA certificate:");
    assert!(field.rect().width() > 100.);
    if let Ok(out) = std::env::var("CODISSEU_RDS_TLS_RENDER") {
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("rds-automatic-ca.png"))
            .unwrap();
    }
    h.get_by_role_and_label(Role::TextInput, "CA certificate:")
        .click();
    h.event(egui::Event::Text("private-ca.pem".into()));
    h.run();
    assert_eq!(h.state().database_ui.config.ca_file, "private-ca.pem");
    assert!(!h.state().database_ui.config.automatic_rds_ca());
    assert!(!h.state().service.remote_db.is_busy());
    assert!(h.state().service.remote_db.active.is_none());
    h.get_by_label("Fechar datasources").click();
    h.run();
    assert!(!h.state().database_ui.dialog);
}

#[test]
fn mongo_options_select_apply_restore_without_connecting_and_keep_existing_engines() {
    for scale in [1., 1.25] {
        let dir = tempfile::tempdir().unwrap();
        let service = AppService::new(dir.path().join("session.json"));
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(100)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        h.state_mut().database_ui.select(ConnectionConfig {
            id: "mongo-options".into(),
            name: "Mongo TLS replica set".into(),
            engine: Engine::Mongo,
            host: "127.0.0.1".into(),
            database: "fixture".into(),
            auth_database: "admin".into(),
            username: "fixture".into(),
            save_credentials: false,
            ..Default::default()
        });
        h.state_mut().database_ui.dialog = true;
        h.run();
        h.get_by_label("Options").click();
        h.run();
        h.get_by_role_and_label(Role::ComboBox, "Authentication:")
            .click();
        h.run();
        h.get_by_label("SCRAM-SHA-1").click();
        h.run();
        h.get_by_role_and_label(Role::TextInput, "Replica set:")
            .click();
        h.event(egui::Event::Text("fixture-rs".into()));
        h.run();
        h.get_by_role_and_label(Role::ComboBox, "Connection mode:")
            .click();
        h.run();
        h.get_by_label("Replica set · descoberta").click();
        h.run();
        h.get_by_role_and_label(Role::ComboBox, "Read preference:")
            .click();
        h.run();
        h.get_by_label("secondaryPreferred").click();
        h.run();
        let options = h.state().database_ui.config.mongo.clone();
        assert_eq!(options.authentication, MongoAuthentication::ScramSha1);
        assert_eq!(options.replica_set, "fixture-rs");
        assert!(!options.direct_connection);
        assert_eq!(
            options.read_preference,
            MongoReadPreference::SecondaryPreferred
        );
        if let Ok(out) = std::env::var("CODISSEU_MONGO_UI_CAPTURES") {
            std::fs::create_dir_all(&out).unwrap();
            h.event(egui::Event::PointerGone);
            h.run();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("mongo-options-{scale}.png")))
                .unwrap();
        }
        h.get_by_label("Apply").click();
        settle(&mut h);
        assert_eq!(
            h.state()
                .service
                .remote_db
                .profiles
                .iter()
                .find(|c| c.id == "mongo-options")
                .unwrap()
                .mongo,
            options
        );
        assert!(h.state().service.remote_db.active.is_none());
        assert!(h.state().service.remote_db.objects.is_empty());
        h.state_mut().database_ui.select(ConnectionConfig {
            id: "sql-options".into(),
            name: "SQL untouched".into(),
            save_credentials: false,
            ..Default::default()
        });
        h.state_mut().database_ui.options = true;
        h.run();
        assert!(
            h.query_by_role_and_label(Role::TextInput, "Replica set:")
                .is_none()
        );
        assert!(
            h.query_by_role_and_label(Role::ComboBox, "Read preference:")
                .is_none()
        );
        drop(h);
        let mut restored = AppService::new(dir.path().join("session.json"));
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            restored.poll();
            if !restored.is_busy() {
                break;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            restored
                .remote_db
                .profiles
                .iter()
                .find(|c| c.id == "mongo-options")
                .unwrap()
                .mongo,
            options
        );
        assert!(restored.remote_db.active.is_none());
    }
}

#[test]
fn imported_certificate_survives_missing_source_and_can_return_to_automatic() {
    for scale in [1., 1.25] {
        let (dir, mut h) = harness_scaled(scale);
        let source = dir.path().join("company-ca.pem");
        std::fs::write(
            &source,
            include_bytes!("../../codisseu-db/certificates/aws-rds-roots.pem"),
        )
        .unwrap();
        h.state_mut().database_ui.select(ConnectionConfig {
            id: "portable-ca".into(),
            name: "Own imported CA fixture".into(),
            engine: Engine::Mongo,
            host: "fixture.internal".into(),
            database: "fixture".into(),
            ca_file: source.to_string_lossy().into_owned(),
            save_credentials: false,
            ..Default::default()
        });
        h.state_mut().database_ui.dialog = true;
        h.state_mut().database_ui.ssl = true;
        h.run();
        h.get_by_label("Apply").click();
        settle(&mut h);
        let saved = h
            .state()
            .service
            .remote_db
            .profiles
            .iter()
            .find(|c| c.id == "portable-ca")
            .unwrap()
            .clone();
        assert!(saved.ca_file.is_empty());
        assert!(!saved.ca_pem.is_empty());
        std::fs::remove_file(&source).unwrap();
        h.state_mut().database_ui.select(saved);
        h.state_mut().database_ui.ssl = true;
        h.run();
        assert!(h.query_by_label("Automático").is_some());
        if let Ok(out) = std::env::var("CODISSEU_PORTABLE_CA_CAPTURES") {
            std::fs::create_dir_all(&out).unwrap();
            h.event(egui::Event::PointerGone);
            h.run();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("stored-ca-{scale}.png")))
                .unwrap();
        }
        h.get_by_label("Automático").click();
        h.run();
        h.get_by_label("Apply").click();
        settle(&mut h);
        let saved = h
            .state()
            .service
            .remote_db
            .profiles
            .iter()
            .find(|c| c.id == "portable-ca")
            .unwrap();
        assert!(saved.ca_pem.is_empty() && saved.ca_file.is_empty());
        assert!(h.state().service.remote_db.active.is_none());
    }
}
