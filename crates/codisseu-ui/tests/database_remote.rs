//! Datasource widgets use the real worker and preserve non-secret profiles.
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

fn field(ui: &mut Harness<'_, CodisseuApp>, label: &str, text: &str) {
    let role = if label == "Password:" {
        Role::PasswordInput
    } else {
        Role::TextInput
    };
    ui.get_by_role_and_label(role, label).scroll_to_me();
    settle(ui);
    ui.get_by_role_and_label(role, label).click();
    settle(ui);
    assert!(
        ui.get_by_role_and_label(role, label).is_focused(),
        "{label} was not focused by click at {:?}",
        ui.get_by_role_and_label(role, label).rect()
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(text.into()));
    settle(ui);
}

#[test]
#[ignore = "requires task-owned codisseu-review-mysql-20261029 and CODISSEU_REVIEW_MYSQL_PORT"]
fn mysql_gui_reuses_one_server_session_across_queries_pages_and_tabs() {
    fn local_sql(sql: &str) -> String {
        let output = std::process::Command::new("docker")
            .args([
                "exec",
                "codisseu-review-mysql-20261029",
                "mysql",
                "-uroot",
                "-pcodisseu_fixture",
                "-N",
                "-B",
                "-e",
                sql,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    let port = std::env::var("CODISSEU_REVIEW_MYSQL_PORT")
        .unwrap()
        .parse()
        .unwrap();
    // Explicit fixture identity; refuse any pre-existing container with a different owner.
    let label = std::process::Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{index .Config.Labels \"codisseu.fixture\"}}",
            "codisseu-review-mysql-20261029",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(label.stdout).unwrap().trim(),
        "connection-review-20261029"
    );
    local_sql(
        "CREATE USER IF NOT EXISTS 'codisseu_ui_review'@'%' IDENTIFIED BY 'codisseu_fixture'; GRANT SELECT ON codisseu.* TO 'codisseu_ui_review'@'%';",
    );
    let count = |stage: &str, expected: usize| {
        let rows = local_sql(
            "SELECT ID FROM information_schema.PROCESSLIST WHERE USER='codisseu_ui_review' ORDER BY ID",
        );
        let ids: Vec<_> = rows.lines().filter(|s| !s.is_empty()).collect();
        println!("GUI {stage}: sessions={}, ids={ids:?}", ids.len());
        assert_eq!(ids.len(), expected, "{stage}");
        rows
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("review.sql");
    std::fs::write(&file, "SELECT * FROM connection_fixture ORDER BY id").unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service.dispatch(Command::OpenFile(file)).unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 900.])
        .with_max_steps(80)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    count("open SQL tab", 0);
    let config = ConnectionConfig {
        id: "ui-review".into(),
        name: "Local review only".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: "codisseu_ui_review".into(),
        tls: false,
        save_credentials: false,
        read_only: true,
        row_limit: 2,
        idle_secs: 60,
        ..Default::default()
    };
    ui.state_mut()
        .service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Save(config)))
        .unwrap();
    settle(&mut ui);
    ui.get_by_label("Data Sources and Drivers").click();
    settle(&mut ui);
    settle(&mut ui);
    field(&mut ui, "Password:", "codisseu_fixture");
    ui.get_by_label("Test Connection").click();
    settle(&mut ui);
    count("test connection", 0);
    ui.get_by_label("Connect").click();
    settle(&mut ui);
    let identity = count("Connect button", 1);
    for index in 0..10 {
        ui.get_by_role(Role::MultilineTextInput).click();
        settle(&mut ui);
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
            2
        );
        ui.get_by_label("Próxima página").click();
        settle(&mut ui);
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
        ui.get_by_label("Página anterior").click();
        settle(&mut ui);
        assert_eq!(
            count(&format!("execute/next/previous {index}"), 1),
            identity
        );
    }
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_CAPTURES") {
        std::fs::create_dir_all(&out).unwrap();
        ui.render()
            .unwrap()
            .save(std::path::PathBuf::from(out).join("database-one-session.png"))
            .unwrap();
    }
    let id = ui.state().service.active.unwrap();
    ui.state_mut()
        .service
        .dispatch(Command::Close { id, discard: false })
        .unwrap();
    settle(&mut ui);
    assert_eq!(
        count("close query tab (datasource still connected)", 1),
        identity
    );
    ui.get_by_label("Desconectar datasource").click();
    settle(&mut ui);
    count("Disconnect button", 0);
    drop(ui);
    count("close app", 0);
}

#[test]
fn datasource_modal_applies_non_secret_profile_and_cancel_does_not_connect() {
    for size in [[1440., 1000.], [1280., 800.]] {
        let directory = tempfile::tempdir().unwrap();
        let session_path = directory.path().join("session.json");
        let mut ui = Harness::builder()
            .with_size(size)
            .with_max_steps(50)
            .build_eframe(|_| CodisseuApp::new(AppService::new(session_path.clone())));
        settle(&mut ui);
        ui.get_by_label("Data Sources and Drivers").click();
        settle(&mut ui);
        field(&mut ui, "Name:", "Company fixture");
        field(&mut ui, "Host:", "fixture.example.invalid");
        field(&mut ui, "User:", "fixture_reader");
        field(&mut ui, "Password:", "never-persist-this-password");
        field(&mut ui, "Database:", "fixture_database");
        ui.get_by_label("Lembrar senha no cofre do sistema").click();
        settle(&mut ui);
        ui.get_by_label("Never").click();
        settle(&mut ui);
        ui.get_by_label("Options").scroll_to_me();
        settle(&mut ui);
        ui.get_by_label("Options").click();
        settle(&mut ui);
        ui.get_by_label("Read-only · somente leitura");
        ui.get_by_label("Apply").click();
        settle(&mut ui);
        assert!(ui.state().service.remote_db.error.is_none());
        assert!(ui.state().service.remote_db.active.is_none());
        let profiles = &ui.state().service.remote_db.profiles;
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Company fixture");
        assert_eq!(profiles[0].host, "fixture.example.invalid");
        assert_eq!(profiles[0].database, "fixture_database");
        assert!(profiles[0].read_only);
        let saved =
            std::fs::read_to_string(session_path.with_extension("datasources.json")).unwrap();
        assert!(saved.contains("fixture_reader"));
        assert!(!saved.contains("never-persist-this-password"));
        assert!(!saved.contains("password"));
        ui.get_by_label("Cancel").click();
        settle(&mut ui);
        assert!(ui.query_by_label("Test Connection").is_none());
        assert!(ui.state().service.remote_db.active.is_none());
        drop(ui);
        let mut reopened = AppService::new(session_path);
        let deadline = Instant::now() + Duration::from_secs(3);
        while reopened.remote_db.profiles.is_empty() {
            reopened.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(reopened.remote_db.profiles[0].name, "Company fixture");
        assert!(reopened.remote_db.active.is_none());
    }
}

#[test]
#[ignore = "requires disposable local Redis; CODISSEU_TEST_REDIS_PORT"]
fn redis_console_widget_ctrl_enter_returns_real_data_and_disconnects() {
    let port: u16 = std::env::var("CODISSEU_TEST_REDIS_PORT")
        .expect("Point CODISSEU_TEST_REDIS_PORT to a disposable local Redis")
        .parse()
        .unwrap();
    // Seed exactly one explicit key through the documented wire protocol.
    {
        use std::io::{Read, Write};
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .write_all(b"*3\r\n$3\r\nSET\r\n$11\r\ncodisseu:ui\r\n$12\r\nreal fixture\r\n")
            .unwrap();
        let mut response = [0; 5];
        socket.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"+OK\r\n");
    }
    let directory = tempfile::tempdir().unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !service.remote_db.poll() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config: ConnectionConfig {
                id: "ui-redis".into(),
                name: "Redis UI fixture".into(),
                engine: Engine::Redis,
                host: "127.0.0.1".into(),
                port,
                tls: false,
                timeout_secs: 3,
                idle_secs: 60,
                ..Default::default()
            },
            password: String::new(),
            test: false,
        }))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 1000.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    assert!(ui.state().service.remote_db.active.is_some());
    ui.get_by_label("Console").click();
    settle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("GET codisseu:ui".into()));
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(
        ui.state().service.remote_db.error.is_none(),
        "{:?}",
        ui.state().service.remote_db.error
    );
    assert_eq!(
        ui.state().service.remote_db.result.as_ref().unwrap().rows[0][1],
        Cell::Text("real fixture".into())
    );
    assert!(
        ui.query_by_role_and_label(Role::Cell, "real fixture")
            .is_some()
    );
    ui.get_by_label("Desconectar datasource").click();
    settle(&mut ui);
    assert!(ui.state().service.remote_db.active.is_none());
}

#[test]
#[ignore = "requires disposable local Redis; CODISSEU_TEST_REDIS_PORT"]
fn redis_console_binding_survives_restart_and_never_falls_back_to_sqlite() {
    let port: u16 = std::env::var("CODISSEU_TEST_REDIS_PORT")
        .expect("Point CODISSEU_TEST_REDIS_PORT to a disposable local Redis")
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let session_path = directory.path().join("session.json");
    let mut service = AppService::new(session_path.clone());
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.remote_db.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    service
        .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
            config: ConnectionConfig {
                id: "redis-bound-console".into(),
                name: "Bound Redis fixture".into(),
                engine: Engine::Redis,
                host: "127.0.0.1".into(),
                port,
                tls: false,
                timeout_secs: 3,
                idle_secs: 60,
                ..Default::default()
            },
            password: String::new(),
            test: false,
        }))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 1000.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    assert!(ui.state().service.remote_db.active.is_some());
    ui.get_by_label("Console").click();
    settle(&mut ui);
    let console = ui.state().service.active_document().unwrap().path.clone();
    assert_eq!(
        ui.state()
            .service
            .remote_db
            .console_target(&console)
            .unwrap(),
        Some("redis-bound-console")
    );
    assert!(ui.state().service.remote_db.is_console_path(&console));
    ui.get_by_label("Desconectar datasource").click();
    settle(&mut ui);
    assert!(ui.state().service.remote_db.active.is_none());
    drop(ui);

    // Read the binding again from disk through a fresh application worker.
    let mut reopened = AppService::new(session_path);
    let deadline = Instant::now() + Duration::from_secs(3);
    while reopened.remote_db.console_target(&console).ok().flatten() != Some("redis-bound-console")
    {
        reopened.poll();
        assert!(
            Instant::now() < deadline,
            "Console binding was not restored"
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(reopened.remote_db.active.is_none());
    let profile = reopened
        .remote_db
        .profiles
        .iter()
        .find(|p| p.id == "redis-bound-console")
        .unwrap();
    assert!(profile.read_only);
    assert_eq!(profile.engine, Engine::Redis);
    reopened
        .dispatch(Command::OpenFile(console.clone()))
        .unwrap();
    let sqlite = directory.path().join("other-database.sqlite");
    drop(rusqlite::Connection::open(&sqlite).unwrap());
    reopened
        .dispatch(Command::Database(codisseu_core::DbCommand::Connect(sqlite)))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 1000.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(reopened));
    settle(&mut ui);
    assert!(ui.state().service.db.connected);
    assert_eq!(ui.state().service.active_document().unwrap().path, console);
    ui.state_mut()
        .service
        .dispatch(Command::Database(codisseu_core::DbCommand::Execute {
            sql: "SELECT 7 AS sqlite_marker".into(),
        }))
        .unwrap();
    settle(&mut ui);
    let previous_revision = ui.state().service.db.result_revision();
    let previous_rows = ui.state().service.db.result.as_ref().unwrap().rows.clone();
    assert_eq!(
        previous_rows,
        vec![vec![rusqlite::types::Value::Integer(7)]]
    );

    // This text is valid SQLite, making an accidental fallback observable.
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text("SELECT 99 AS wrong_database".into()));
    settle(&mut ui);
    assert_eq!(
        ui.state().service.active_document().unwrap().text(),
        "SELECT 99 AS wrong_database"
    );
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    let error = ui.state().service.remote_db.error.as_deref().unwrap();
    assert!(error.contains("datasource"), "{error}");
    assert!(ui.state().service.remote_db.active.is_none());
    assert!(ui.state().service.db.last_error.is_none());
    assert_eq!(ui.state().service.db.result_revision(), previous_revision);
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows,
        previous_rows
    );
    assert!(ui.state().service.remote_db.result.is_none());
}
