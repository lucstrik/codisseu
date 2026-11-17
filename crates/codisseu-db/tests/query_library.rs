use codisseu_db::remote::{ConnectionConfig, RemoteCommand, RemoteService};
use std::{
    path::Path,
    time::{Duration, Instant},
};
fn settle(service: &mut RemoteService) {
    let until = Instant::now() + Duration::from_secs(5);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn service(root: &Path) -> RemoteService {
    let mut s = RemoteService::new(root.join("sources.json"));
    settle(&mut s);
    s
}
fn source(s: &mut RemoteService) {
    s.dispatch(RemoteCommand::Save(ConnectionConfig {
        id: "fixture".into(),
        name: "Fixture".into(),
        save_credentials: false,
        ..Default::default()
    }))
    .unwrap();
    settle(s);
}
fn create(s: &mut RemoteService) -> std::path::PathBuf {
    s.dispatch(RemoteCommand::CreateConsoleFor("fixture".into()))
        .unwrap();
    settle(s);
    assert!(s.error.is_none(), "{:?}", s.error);
    s.console_path.take().unwrap()
}
#[test]
fn query_names_legacy_index_restart_and_deleted_source_preserve_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path());
    source(&mut s);
    let first = create(&mut s);
    assert_eq!(first.file_name().unwrap(), "query-1.sql");
    let second = create(&mut s);
    assert_eq!(second.file_name().unwrap(), "query-2.sql");
    std::fs::write(&first, "SELECT 'saved';\n").unwrap();
    s.dispatch(RemoteCommand::RenameConsole {
        path: first.clone(),
        name: "Pedidos março".into(),
    })
    .unwrap();
    settle(&mut s);
    assert!(s.error.is_none(), "{:?}", s.error);
    let renamed = s.console_renamed.take().unwrap().path;
    assert_eq!(renamed.file_name().unwrap(), "Pedidos março.sql");
    assert!(!first.exists());
    assert_eq!(
        std::fs::read_to_string(&renamed).unwrap(),
        "SELECT 'saved';\n"
    );
    s.dispatch(RemoteCommand::DeleteRemembered("fixture".into()))
        .unwrap();
    settle(&mut s);
    assert!(s.profiles.is_empty());
    assert_eq!(s.console_bindings.len(), 2);
    assert!(renamed.exists());
    drop(s);
    let s = service(dir.path());
    assert!(s.profiles.is_empty());
    assert!(s.console_bindings.contains_key(&renamed));
    assert!(s.console_bindings.contains_key(&second));
    assert_eq!(s.console_target(&renamed).unwrap(), Some("fixture"));
}
#[test]
fn query_rename_rejects_collision_traversal_symlink_and_index_failure_without_loss() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path());
    source(&mut s);
    let first = create(&mut s);
    let second = create(&mut s);
    let original = std::fs::read(&first).unwrap();
    for name in ["query-2", "../escape", "a/b", "", ".."] {
        match s.dispatch(RemoteCommand::RenameConsole {
            path: first.clone(),
            name: name.into(),
        }) {
            Ok(()) => settle(&mut s),
            Err(_) => continue,
        };
        assert!(s.error.is_some());
        assert_eq!(std::fs::read(&first).unwrap(), original);
        assert!(second.exists());
    }
    std::fs::remove_file(dir.path().join("sources.console-bindings.json")).unwrap();
    std::fs::create_dir(dir.path().join("sources.console-bindings.json")).unwrap();
    s.dispatch(RemoteCommand::RenameConsole {
        path: first.clone(),
        name: "index-failure".into(),
    })
    .unwrap();
    settle(&mut s);
    assert!(s.error.is_some());
    assert!(first.exists());
    assert!(!first.with_file_name("index-failure.sql").exists());
    assert!(s.console_bindings.contains_key(&first));
    #[cfg(unix)]
    {
        std::fs::remove_file(&first).unwrap();
        std::os::unix::fs::symlink(&second, &first).unwrap();
        s.dispatch(RemoteCommand::RenameConsole {
            path: first.clone(),
            name: "linked".into(),
        })
        .unwrap();
        settle(&mut s);
        assert!(s.error.is_some());
        assert!(!first.with_file_name("linked.sql").exists());
    }
}
#[test]
fn legacy_console_names_remain_indexed_and_unowned_files_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let consoles = dir.path().join("consoles");
    std::fs::create_dir(&consoles).unwrap();
    let legacy = consoles.join("console-1793110532735409428.sql");
    std::fs::write(&legacy, "SELECT 42;").unwrap();
    std::fs::write(consoles.join("query-1.sql"), "preserve").unwrap();
    std::fs::write(
        dir.path().join("sources.console-bindings.json"),
        serde_json::to_vec(&std::collections::BTreeMap::from([(
            legacy.clone(),
            "fixture",
        )]))
        .unwrap(),
    )
    .unwrap();
    let mut s = service(dir.path());
    source(&mut s);
    assert!(s.console_bindings.contains_key(&legacy));
    let new = create(&mut s);
    // An unindexed flat file is not owned by this datasource. New queries use
    // its dedicated directory without touching that unrelated file.
    assert_eq!(new.file_name().unwrap(), "query-1.sql");
    assert_ne!(new.parent(), Some(consoles.as_path()));
    assert_eq!(
        std::fs::read_to_string(consoles.join("query-1.sql")).unwrap(),
        "preserve"
    );
    assert!(legacy.exists());
}

#[test]
fn deleting_active_source_closes_owned_socket_and_keeps_query_file() {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (done, closed) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut input = BufReader::new(socket.try_clone().unwrap());
        let mut line = String::new();
        input.read_line(&mut line).unwrap();
        assert_eq!(line, "version\r\n");
        socket.write_all(b"VERSION fixture\r\n").unwrap();
        line.clear();
        done.send(input.read_line(&mut line).unwrap() == 0).unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path());
    s.dispatch(RemoteCommand::Connect {
        config: ConnectionConfig {
            id: "fixture".into(),
            name: "Fixture socket".into(),
            engine: codisseu_db::remote::Engine::Memcached,
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
    settle(&mut s);
    assert!(s.error.is_none(), "{:?}", s.error);
    let file = create(&mut s);
    s.dispatch(RemoteCommand::DeleteRemembered("fixture".into()))
        .unwrap();
    settle(&mut s);
    assert!(s.active.is_none());
    assert!(s.profiles.is_empty());
    assert!(file.exists());
    assert!(s.console_bindings.contains_key(&file));
    assert!(closed.recv_timeout(Duration::from_secs(2)).unwrap());
    server.join().unwrap();
}

#[test]
fn query_delete_removes_only_selected_file_and_binding_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path());
    source(&mut s);
    let first = create(&mut s);
    let second = create(&mut s);
    s.dispatch(RemoteCommand::DeleteConsole(first.clone()))
        .unwrap();
    assert!(s.console_delete_pending());
    settle(&mut s);
    assert!(s.error.is_none(), "{:?}", s.error);
    assert_eq!(s.console_deleted.take(), Some(first.clone()));
    assert!(!first.exists());
    assert!(second.exists());
    assert!(!s.console_bindings.contains_key(&first));
    assert_eq!(s.profiles.len(), 1);
    drop(s);
    let s = service(dir.path());
    assert!(!s.console_bindings.contains_key(&first));
    assert!(s.console_bindings.contains_key(&second));
}
#[test]
fn query_delete_rejects_index_failure_external_path_and_symlink_preserving_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = service(dir.path());
    source(&mut s);
    let file = create(&mut s);
    let original = std::fs::read(&file).unwrap();
    let outside = dir.path().join("external.sql");
    std::fs::write(&outside, "preserve").unwrap();
    assert!(
        s.dispatch(RemoteCommand::DeleteConsole(outside.clone()))
            .is_err()
    );
    std::fs::remove_file(dir.path().join("sources.console-bindings.json")).unwrap();
    std::fs::create_dir(dir.path().join("sources.console-bindings.json")).unwrap();
    s.dispatch(RemoteCommand::DeleteConsole(file.clone()))
        .unwrap();
    settle(&mut s);
    assert!(s.error.is_some());
    assert_eq!(std::fs::read(&file).unwrap(), original);
    assert!(s.console_bindings.contains_key(&file));
    #[cfg(unix)]
    {
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&outside, &file).unwrap();
        s.dispatch(RemoteCommand::DeleteConsole(file.clone()))
            .unwrap();
        settle(&mut s);
        assert!(s.error.is_some());
        assert!(file.symlink_metadata().unwrap().file_type().is_symlink());
    }
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "preserve");
}
