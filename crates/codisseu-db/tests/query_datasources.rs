use codisseu_db::remote::{ConnectionConfig, Engine, RemoteCommand, RemoteService};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn settle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{:?}", service.error);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn start_service(root: &Path) -> RemoteService {
    let mut service = RemoteService::new(root.join("sources.json"));
    settle(&mut service);
    service
}
fn save(service: &mut RemoteService, id: &str, name: &str, engine: Engine) {
    service
        .dispatch(RemoteCommand::Save(ConnectionConfig {
            id: id.into(),
            name: name.into(),
            engine,
            save_credentials: false,
            ..Default::default()
        }))
        .unwrap();
    settle(service);
    assert!(service.error.is_none(), "{:?}", service.error);
}
fn create(service: &mut RemoteService, id: &str) -> PathBuf {
    service
        .dispatch(RemoteCommand::CreateConsoleFor(id.into()))
        .unwrap();
    settle(service);
    assert!(service.error.is_none(), "{:?}", service.error);
    assert!(service.active.is_none(), "File operations must not connect");
    service.console_path.take().unwrap()
}
fn rejected(service: &mut RemoteService, command: RemoteCommand) {
    if service.dispatch(command).is_ok() {
        settle(service);
        assert!(service.error.is_some(), "Command unexpectedly succeeded");
    }
}
fn seed_index(root: &Path, bindings: &BTreeMap<PathBuf, String>) {
    std::fs::write(
        root.join("sources.console-bindings.json"),
        serde_json::to_vec(bindings).unwrap(),
    )
    .unwrap();
}

#[test]
fn each_datasource_has_its_own_files_numbering_and_engine_extension_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let mut service = start_service(root.path());
    let sources = [
        (
            "dev",
            "ExampleGroup DEV",
            Engine::Mysql,
            "sql",
            "SELECT 1;\n",
        ),
        ("qa", "ExampleGroup QA", Engine::Mysql, "sql", "SELECT 1;\n"),
        (
            "prod",
            "ExampleGroup PROD",
            Engine::Postgres,
            "sql",
            "SELECT 1;\n",
        ),
        ("cache", "Redis DEV", Engine::Redis, "txt", "PING\n"),
        (
            "docs",
            "Mongo DEV",
            Engine::Mongo,
            "json",
            "{\"collection\":\"users\",\"filter\":{},\"limit\":100}\n",
        ),
    ];
    let mut files = Vec::new();
    for (id, name, engine, extension, sample) in sources {
        save(&mut service, id, name, engine);
        let file = create(&mut service, id);
        assert_eq!(
            file.file_name().unwrap(),
            format!("query-1.{extension}").as_str()
        );
        assert_eq!(
            file.parent().unwrap().parent().unwrap(),
            root.path().join("consoles")
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), sample);
        assert_eq!(service.console_target(&file).unwrap(), Some(id));
        assert!(
            files
                .iter()
                .all(|previous: &PathBuf| previous.parent() != file.parent())
        );
        files.push(file);
    }
    drop(service);
    let mut reopened = start_service(root.path());
    assert_eq!(reopened.console_bindings.len(), 5);
    for ((id, _, _, extension, _), file) in sources.into_iter().zip(&files) {
        assert_eq!(reopened.console_target(file).unwrap(), Some(id));
        let second = create(&mut reopened, id);
        assert_eq!(
            second.file_name().unwrap(),
            format!("query-2.{extension}").as_str()
        );
        assert_eq!(second.parent(), file.parent());
    }
}

#[test]
fn datasource_rename_keeps_stable_namespace_and_query_rename_delete_are_isolated() {
    let root = tempfile::tempdir().unwrap();
    let mut service = start_service(root.path());
    // An opaque ID can contain characters unsuitable for a directory name.
    let dev = "../DEV:/日本語";
    save(&mut service, dev, "ExampleGroup", Engine::Mysql);
    save(&mut service, "qa", "ExampleGroup", Engine::Mysql);
    let dev_file = create(&mut service, dev);
    let qa_file = create(&mut service, "qa");
    std::fs::write(&dev_file, "SELECT 'DEV';").unwrap();
    std::fs::write(&qa_file, "SELECT 'QA';").unwrap();
    save(
        &mut service,
        dev,
        "ExampleGroup DEV renomeado",
        Engine::Mysql,
    );
    let second = create(&mut service, dev);
    assert_eq!(second.parent(), dev_file.parent());
    assert_eq!(second.file_name().unwrap(), "query-2.sql");
    for file in [&dev_file, &qa_file] {
        service
            .dispatch(RemoteCommand::RenameConsole {
                path: file.clone(),
                name: "Pedidos".into(),
            })
            .unwrap();
        settle(&mut service);
        assert!(service.error.is_none(), "{:?}", service.error);
    }
    let dev_named = dev_file.with_file_name("Pedidos.sql");
    let qa_named = qa_file.with_file_name("Pedidos.sql");
    service
        .dispatch(RemoteCommand::DeleteConsole(dev_named.clone()))
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    assert!(!dev_named.exists());
    assert_eq!(std::fs::read_to_string(&qa_named).unwrap(), "SELECT 'QA';");
    service
        .dispatch(RemoteCommand::Delete("qa".into()))
        .unwrap();
    settle(&mut service);
    assert_eq!(service.console_target(&qa_named).unwrap(), Some("qa"));
    assert!(qa_named.exists());
    assert!(second.exists());
    drop(service);
    let reopened = start_service(root.path());
    assert!(!reopened.console_bindings.contains_key(&dev_named));
    assert_eq!(reopened.console_target(&qa_named).unwrap(), Some("qa"));
    assert_eq!(reopened.console_target(&second).unwrap(), Some(dev));
}

#[test]
fn flat_legacy_files_keep_their_paths_bytes_and_owner_local_name_reservations() {
    let root = tempfile::tempdir().unwrap();
    let consoles = root.path().join("consoles");
    std::fs::create_dir(&consoles).unwrap();
    let legacy = consoles.join("query-1.sql");
    let old_timestamp = consoles.join("console-1793110532735409428.sql");
    std::fs::write(&legacy, "SELECT 'old DEV';").unwrap();
    std::fs::write(&old_timestamp, "SELECT 'old QA';").unwrap();
    seed_index(
        root.path(),
        &BTreeMap::from([
            (legacy.clone(), "dev".into()),
            (old_timestamp.clone(), "qa".into()),
        ]),
    );
    let mut service = start_service(root.path());
    save(&mut service, "dev", "DEV", Engine::Mysql);
    save(&mut service, "qa", "QA", Engine::Postgres);
    let dev = create(&mut service, "dev");
    let qa = create(&mut service, "qa");
    assert_eq!(dev.file_name().unwrap(), "query-2.sql");
    assert_eq!(qa.file_name().unwrap(), "query-1.sql");
    assert_eq!(
        std::fs::read_to_string(&legacy).unwrap(),
        "SELECT 'old DEV';"
    );
    assert_eq!(
        std::fs::read_to_string(&old_timestamp).unwrap(),
        "SELECT 'old QA';"
    );
    rejected(
        &mut service,
        RemoteCommand::RenameConsole {
            path: dev.clone(),
            name: "query-1".into(),
        },
    );
    rejected(
        &mut service,
        RemoteCommand::RenameConsole {
            path: old_timestamp.clone(),
            name: "query-1".into(),
        },
    );
    assert!(dev.exists() && qa.exists() && old_timestamp.exists());
    service
        .dispatch(RemoteCommand::RenameConsole {
            path: legacy.clone(),
            name: "Legado".into(),
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    let renamed = service.console_renamed.take().unwrap().path;
    assert_eq!(renamed.file_name().unwrap(), "Legado.sql");
    assert_eq!(renamed.parent(), dev.parent());
    assert_eq!(
        std::fs::read_to_string(&renamed).unwrap(),
        "SELECT 'old DEV';"
    );
    assert_eq!(service.console_target(&renamed).unwrap(), Some("dev"));
    service
        .dispatch(RemoteCommand::DeleteConsole(old_timestamp.clone()))
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    assert!(!old_timestamp.exists());
    assert!(renamed.exists() && dev.exists() && qa.exists());
}

#[test]
fn explicit_legacy_rename_can_reuse_another_datasources_flat_filename() {
    let root = tempfile::tempdir().unwrap();
    let consoles = root.path().join("consoles");
    std::fs::create_dir(&consoles).unwrap();
    let dev = consoles.join("query-1.sql");
    let qa = consoles.join("query-2.sql");
    std::fs::write(&dev, "SELECT 'DEV';").unwrap();
    std::fs::write(&qa, "SELECT 'QA';").unwrap();
    seed_index(
        root.path(),
        &BTreeMap::from([(dev.clone(), "dev".into()), (qa.clone(), "qa".into())]),
    );
    let mut service = start_service(root.path());
    // Opening the index never migrates either legacy path.
    assert!(dev.exists() && qa.exists());
    let target = service.console_rename_target(&qa, "query-1").unwrap();
    assert_ne!(target, dev);
    assert_eq!(target.file_name().unwrap(), "query-1.sql");
    service
        .dispatch(RemoteCommand::RenameConsole {
            path: qa.clone(),
            name: "query-1".into(),
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    let receipt = service.console_renamed.take().unwrap();
    assert_eq!(receipt.source, qa);
    assert_eq!(receipt.path, target);
    assert_eq!(target.parent().unwrap().parent(), Some(consoles.as_path()));
    assert_eq!(std::fs::read_to_string(&dev).unwrap(), "SELECT 'DEV';");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "SELECT 'QA';");
    assert!(!qa.exists());
    assert_eq!(service.console_target(&dev).unwrap(), Some("dev"));
    assert_eq!(service.console_target(&target).unwrap(), Some("qa"));
    drop(service);
    let reopened = start_service(root.path());
    assert!(!reopened.console_bindings.contains_key(&qa));
    assert_eq!(reopened.console_target(&dev).unwrap(), Some("dev"));
    assert_eq!(reopened.console_target(&target).unwrap(), Some("qa"));
    assert!(reopened.active.is_none());
}

#[test]
fn forged_cross_datasource_namespace_rejects_rename_and_delete_without_touching_file() {
    let root = tempfile::tempdir().unwrap();
    let mut service = start_service(root.path());
    save(&mut service, "dev", "DEV", Engine::Mysql);
    save(&mut service, "qa", "QA", Engine::Mysql);
    let file = create(&mut service, "qa");
    let before = std::fs::read(&file).unwrap();
    drop(service);
    seed_index(root.path(), &BTreeMap::from([(file.clone(), "dev".into())]));
    let mut reopened = start_service(root.path());
    rejected(
        &mut reopened,
        RemoteCommand::RenameConsole {
            path: file.clone(),
            name: "escape".into(),
        },
    );
    rejected(&mut reopened, RemoteCommand::DeleteConsole(file.clone()));
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert!(!file.with_file_name("escape.sql").exists());
}

#[test]
fn bound_sql_query_cannot_execute_on_another_active_datasource_or_engine() {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "version\r\n");
        socket.write_all(b"VERSION fixture\r\n").unwrap();
        line.clear();
        assert_eq!(
            reader.read_line(&mut line).unwrap(),
            0,
            "Wrong-source query reached the server"
        );
    });
    let root = tempfile::tempdir().unwrap();
    let mut service = start_service(root.path());
    save(&mut service, "sql-dev", "ExampleGroup DEV", Engine::Mysql);
    let file = create(&mut service, "sql-dev");
    service
        .dispatch(RemoteCommand::Connect {
            config: ConnectionConfig {
                id: "cache".into(),
                name: "Cache".into(),
                engine: Engine::Memcached,
                host: "127.0.0.1".into(),
                port,
                tls: false,
                save_credentials: false,
                ..Default::default()
            },
            password: String::new(),
            test: false,
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    let owner = service.console_target(&file).unwrap().unwrap().to_owned();
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteBound {
                sql: "SELECT 'DEV';".into(),
                datasource_id: owner
            })
            .is_err()
    );
    assert!(!service.is_busy());
    service.dispatch(RemoteCommand::Disconnect).unwrap();
    settle(&mut service);
    server.join().unwrap();
    assert!(file.exists());
    assert_eq!(service.console_target(&file).unwrap(), Some("sql-dev"));
}

#[cfg(unix)]
#[test]
fn symlink_namespace_or_file_cannot_redirect_create_rename_or_delete() {
    let root = tempfile::tempdir().unwrap();
    let mut service = start_service(root.path());
    save(&mut service, "dev", "DEV", Engine::Mysql);
    let file = create(&mut service, "dev");
    let namespace = file.parent().unwrap().to_owned();
    let real = root.path().join("parked");
    std::fs::rename(&namespace, &real).unwrap();
    std::os::unix::fs::symlink(&real, &namespace).unwrap();
    rejected(&mut service, RemoteCommand::CreateConsoleFor("dev".into()));
    rejected(
        &mut service,
        RemoteCommand::RenameConsole {
            path: file.clone(),
            name: "linked".into(),
        },
    );
    rejected(&mut service, RemoteCommand::DeleteConsole(file.clone()));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "SELECT 1;\n");
    assert_eq!(std::fs::read_dir(&real).unwrap().count(), 1);
    std::fs::remove_file(&namespace).unwrap();
    std::fs::rename(&real, &namespace).unwrap();
    let outside = root.path().join("outside.sql");
    std::fs::write(&outside, "outside bytes").unwrap();
    std::fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(&outside, &file).unwrap();
    rejected(
        &mut service,
        RemoteCommand::RenameConsole {
            path: file.clone(),
            name: "linked".into(),
        },
    );
    rejected(&mut service, RemoteCommand::DeleteConsole(file.clone()));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside bytes");
    assert!(file.symlink_metadata().unwrap().file_type().is_symlink());
}
