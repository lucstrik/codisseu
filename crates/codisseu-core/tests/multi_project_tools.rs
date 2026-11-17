//! Owned aggregate workspace: real HTTP, databases and optional Docker.
use codisseu_core::{AppService, Command, DbCommand};
use rusqlite::{Connection, types::Value};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn idle(app: &mut AppService) {
    let start = Instant::now();
    while app.is_busy() || app.docker.is_busy() {
        app.poll();
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "{} {:?}",
            app.status,
            app.last_error
        );
        thread::sleep(Duration::from_millis(2));
    }
    app.poll();
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
    assert!(app.db.last_error.is_none(), "{:?}", app.db.last_error);
    assert!(app.api.error.is_none(), "{:?}", app.api.error);
}

fn aggregate(root: &Path) -> (AppService, Vec<PathBuf>) {
    let projects =
        ["utf8-service", "latin1-service", "windows1252-service"].map(|name| root.join(name));
    for project in &projects {
        fs::create_dir(project).unwrap();
        fs::write(project.join("composer.json"), "{}").unwrap();
    }
    fs::write(projects[0].join("source.php"), "<?php // ação 界\n").unwrap();
    fs::write(projects[1].join("source.php"), b"<?php // a\xe7\xe3o\n").unwrap();
    fs::write(
        projects[2].join("source.php"),
        b"<?php // \x80 \x93a\xe7\xe3o\x94\n",
    )
    .unwrap();
    let mut app = AppService::new(root.join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.into())).unwrap();
    idle(&mut app);
    (app, projects.to_vec())
}

#[test]
fn aggregate_collections_reach_distinct_microservices_and_sqlite_connections_use_explicit_paths() {
    let fixture = tempfile::tempdir().unwrap();
    let (mut app, projects) = aggregate(fixture.path());
    assert_eq!(app.api.project.as_deref(), Some(fixture.path()));
    assert!(!app.db.connected);
    assert!(!app.docker.state.connected);
    let source_bytes: Vec<_> = projects
        .iter()
        .map(|p| fs::read(p.join("source.php")).unwrap())
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let start = Instant::now();
        let mut routes = Vec::new();
        while routes.len() < 3 {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut bytes = [0; 8192];
                    let n = stream.read(&mut bytes).unwrap();
                    let request = std::str::from_utf8(&bytes[..n]).unwrap();
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_string();
                    let body = format!("{{\"service\":\"{path}\",\"message\":\"ação界\"}}");
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                    routes.push(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        start.elapsed() < Duration::from_secs(20),
                        "expected 3 loopback requests"
                    );
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        }
        routes
    });
    for (index, project) in projects.iter().enumerate() {
        app.dispatch(Command::OpenFile(project.join("composer.json")))
            .unwrap();
        idle(&mut app);
        assert_eq!(
            app.active_document().unwrap().project_root(),
            project.as_path()
        );
        assert_eq!(app.api.project.as_deref(), Some(fixture.path()));
        assert_eq!(app.api.workspace.collections.len(), index);
        if index > 0 {
            assert_eq!(
                app.db.config.as_ref(),
                Some(&projects[index - 1].join("fixture.sqlite")),
                "editor activation must not silently reconnect to another database"
            );
        }
        let collection = app
            .api
            .new_collection(project.file_name().unwrap().to_str().unwrap());
        let id = app.api.new_request(&collection, "Health").unwrap();
        app.api.workspace.find_request_mut(&id).unwrap().url = format!("{origin}/service-{index}");
        app.api.send(&id, false).unwrap();
        idle(&mut app);
        let response = &app.api.responses[&id].response;
        assert_eq!(response.status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response.body).unwrap()["service"],
            format!("/service-{index}")
        );
        assert!(response.text().contains("ação界"));
        let database = project.join("fixture.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE owner(id INTEGER PRIMARY KEY, service TEXT, message TEXT);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO owner VALUES(1, ?1, 'ação界')",
                [format!("service-{index}")],
            )
            .unwrap();
        app.dispatch(Command::Database(DbCommand::Connect(database.clone())))
            .unwrap();
        idle(&mut app);
        app.dispatch(Command::Database(DbCommand::Execute {
            sql: "SELECT service, message FROM owner ORDER BY id".into(),
        }))
        .unwrap();
        idle(&mut app);
        assert_eq!(
            app.db.result.as_ref().unwrap().rows,
            vec![vec![
                Value::Text(format!("service-{index}")),
                Value::Text("ação界".into())
            ]]
        );
        assert_eq!(app.db.config.as_ref(), Some(&database));
        assert_eq!(
            fs::read(project.join("source.php")).unwrap(),
            source_bytes[index]
        );
    }
    assert_eq!(
        server.join().unwrap(),
        ["/service-0", "/service-1", "/service-2"]
    );
    assert_eq!(app.api.workspace.collections.len(), 3);
    app.api.save().unwrap();
    idle(&mut app);
    app.dispatch(Command::Database(DbCommand::Disconnect))
        .unwrap();
    idle(&mut app);
    app.api.shutdown();
    idle(&mut app);
}

#[test]
#[ignore = "requires CODISSEU_DOCKER_TEST_CONTAINER containing an explicitly owned full ID"]
fn aggregate_workspace_docker_uses_captured_context_and_owned_container() {
    use codisseu_core::docker::{ContainerAction, DockerCommand};
    let fixture = tempfile::tempdir().unwrap();
    let (mut app, projects) = aggregate(fixture.path());
    let id = std::env::var("CODISSEU_DOCKER_TEST_CONTAINER")
        .expect("explicit owned fixture ID required");
    app.dispatch(Command::Docker(DockerCommand::Refresh))
        .unwrap();
    idle(&mut app);
    assert!(
        app.docker.state.error.is_none(),
        "{:?}",
        app.docker.state.error
    );
    let owned = app
        .docker
        .state
        .containers
        .iter()
        .find(|c| c.id == id)
        .expect("owned fixture missing");
    assert!(owned.name.starts_with("codisseu-docker-test-"));
    let needs_start = owned.state != "running";
    app.dispatch(Command::Docker(DockerCommand::Select(id.clone())))
        .unwrap();
    idle(&mut app);
    if needs_start {
        app.dispatch(Command::Docker(DockerCommand::Action {
            id: id.clone(),
            action: ContainerAction::Start,
        }))
        .unwrap();
        idle(&mut app);
    }
    app.dispatch(Command::Docker(DockerCommand::Select(id.clone())))
        .unwrap();
    idle(&mut app);
    assert!(
        app.docker.state.error.is_none(),
        "{:?}",
        app.docker.state.error
    );
    let context = app.docker.state.context.clone().unwrap();
    for project in projects {
        app.dispatch(Command::OpenFile(project.join("composer.json")))
            .unwrap();
        idle(&mut app);
        assert_eq!(app.docker.state.context.as_ref(), Some(&context));
        assert_eq!(app.docker.state.selected.as_ref(), Some(&id));
        let spec = app.docker.exec_spec(&id, "sh").unwrap();
        assert_eq!(spec.context, context);
        assert!(spec.args.iter().any(|arg| arg == &id));
    }
    app.dispatch(Command::Docker(DockerCommand::Logs { id: id.clone() }))
        .unwrap();
    idle(&mut app);
    assert!(
        app.docker.state.error.is_none(),
        "{:?}",
        app.docker.state.error
    );
    assert!(
        app.docker
            .state
            .logs
            .contains("Codisseu Docker fixture ready")
    );
    assert_eq!(app.workspace.as_deref(), Some(fixture.path()));
    app.api.shutdown();
    idle(&mut app);
}

#[test]
#[ignore = "requires task-owned loopback MySQL 8.0; CODISSEU_MULTI_MYSQL_PORT"]
fn aggregate_workspace_mysql_keeps_the_same_server_connection_when_switching_projects() {
    use codisseu_core::remote::{Cell, ConnectionConfig, Engine, RemoteCommand};
    let port: u16 = std::env::var("CODISSEU_MULTI_MYSQL_PORT")
        .expect("explicit task-owned loopback MySQL port required")
        .parse()
        .unwrap();
    assert_ne!(port, 0);
    let fixture = tempfile::tempdir().unwrap();
    let (mut app, projects) = aggregate(fixture.path());
    let config = ConnectionConfig {
        id: "multi-project-owned-mysql".into(),
        name: "Owned aggregate MySQL fixture".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        username: "root".into(),
        database: "codisseu".into(),
        tls: false,
        read_only: true,
        save_credentials: false,
        timeout_secs: 5,
        idle_secs: 120,
        ..Default::default()
    };
    app.dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
        config: config.clone(),
        password: "codisseu_fixture".into(),
        test: false,
    }))
    .unwrap();
    idle(&mut app);
    assert!(app.remote_db.error.is_none(), "{:?}", app.remote_db.error);
    let mut connection_id = None;
    for project in projects {
        app.dispatch(Command::OpenFile(project.join("composer.json")))
            .unwrap();
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().project_root(), project);
        assert_eq!(app.remote_db.active.as_ref(), Some(&config));
        app.dispatch(Command::RemoteDatabase(RemoteCommand::ExecuteBound {
            sql: "SELECT 1 AS value, CONNECTION_ID() AS session_id, 'ação界' AS message".into(),
            datasource_id: config.id.clone(),
        }))
        .unwrap();
        idle(&mut app);
        assert!(app.remote_db.error.is_none(), "{:?}", app.remote_db.error);
        let result = app.remote_db.result.as_ref().unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], Cell::Number("1".into()));
        assert_eq!(result.rows[0][2], Cell::Text("ação界".into()));
        match &connection_id {
            None => connection_id = Some(result.rows[0][1].clone()),
            Some(expected) => assert_eq!(
                &result.rows[0][1], expected,
                "opening another project must preserve the actual MySQL CONNECTION_ID"
            ),
        }
    }
    app.dispatch(Command::RemoteDatabase(RemoteCommand::Disconnect))
        .unwrap();
    idle(&mut app);
    assert!(app.remote_db.active.is_none());
    app.api.shutdown();
    idle(&mut app);
}
