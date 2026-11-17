use codisseu_db::remote::Cell;
use codisseu_db::remote::{ConnectionConfig, Engine, RemoteCommand, RemoteService};
use std::time::{Duration, Instant};
fn settle(s: &mut RemoteService) {
    let until = Instant::now() + Duration::from_secs(5);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < until, "{}", s.status);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn clipboard_preserves_full_values_and_empty_strings() {
    let text = "é".repeat(1024);
    let cell = Cell::Text(text.clone());
    assert!(cell.display().len() < text.len());
    assert_eq!(cell.clipboard_text(), text);
    assert_eq!(Cell::Text(String::new()).clipboard_text(), "");
    assert_eq!(Cell::Null.clipboard_text(), "NULL");
    assert_eq!(Cell::Binary(vec![0, 255]).clipboard_text(), "0x00FF");
}
#[test]
fn cancelling_connect_closes_the_socket_without_waiting_for_full_timeout() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted, ready) = std::sync::mpsc::channel();
    let (closed, closed_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        let mut b = [0; 256];
        let n = socket.read(&mut b).unwrap();
        assert!(n > 0);
        accepted.send(()).unwrap();
        let n = socket.read(&mut b).unwrap_or(0);
        let _ = socket.write_all(b"VERSION late\r\n");
        closed.send(n == 0).unwrap();
    });
    let dir = tempfile::tempdir().unwrap();
    let mut s = RemoteService::new(dir.path().join("profiles.json"));
    settle(&mut s);
    let c = ConnectionConfig {
        id: "cancel-connect".into(),
        engine: Engine::Memcached,
        port,
        host: "127.0.0.1".into(),
        tls: false,
        timeout_secs: 120,
        ..ConnectionConfig::default()
    };
    s.dispatch(RemoteCommand::Connect {
        config: c,
        password: String::new(),
        test: false,
    })
    .unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    let start = Instant::now();
    s.dispatch(RemoteCommand::Cancel).unwrap();
    settle(&mut s);
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(s.active.is_none());
    assert!(s.error.as_ref().is_some_and(|e| e.contains("cancelada")));
    assert!(closed_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    server.join().unwrap();
}
#[test]
fn metadata_persists_without_connections_or_password_and_delete_is_durable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("datasources.json");
    let mut s = RemoteService::new(path.clone());
    settle(&mut s);
    for engine in Engine::ALL {
        let c = ConnectionConfig {
            id: engine.label().into(),
            name: engine.label().into(),
            engine,
            port: engine.port(),
            ..ConnectionConfig::default()
        };
        s.dispatch(RemoteCommand::Save(c)).unwrap();
        settle(&mut s);
        assert!(s.error.is_none(), "{:?}", s.error);
    }
    assert_eq!(s.profiles.len(), 7);
    assert!(s.active.is_none());
    let json = std::fs::read_to_string(&path).unwrap();
    assert!(!json.contains("password"));
    assert!(!json.contains("senha"));
    let debug = format!(
        "{:?}",
        RemoteCommand::Connect {
            config: s.profiles[0].clone(),
            password: "never-print-this".into(),
            test: true
        }
    );
    assert!(!debug.contains("never-print-this"));
    drop(s);
    let mut s = RemoteService::new(path.clone());
    settle(&mut s);
    assert_eq!(s.profiles.len(), 7);
    assert!(s.active.is_none());
    s.dispatch(RemoteCommand::Delete("Redis".into())).unwrap();
    settle(&mut s);
    assert_eq!(s.profiles.len(), 6);
    drop(s);
    let mut restored = RemoteService::new(path);
    settle(&mut restored);
    assert!(restored.profiles.iter().all(|c| c.engine != Engine::Redis));
    assert!(restored.active.is_none());
}
#[test]
fn invalid_configuration_and_oversized_query_are_rejected_before_network() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = RemoteService::new(dir.path().join("datasources.json"));
    settle(&mut s);
    let mut c = ConnectionConfig {
        id: "fixture".into(),
        ..ConnectionConfig::default()
    };
    c.host = "mysql://user:secret@localhost".into();
    assert!(s.dispatch(RemoteCommand::Save(c)).is_err());
    assert!(
        s.dispatch(RemoteCommand::Execute("x".repeat(256 * 1024 + 1)))
            .is_err()
    );
    assert!(s.active.is_none());
    assert!(!s.is_busy());
}
#[test]
fn unknown_password_field_is_not_loaded_as_a_datasource() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("datasources.json");
    let c = ConnectionConfig {
        id: "fixture".into(),
        ..ConnectionConfig::default()
    };
    let mut value = serde_json::to_value(c).unwrap();
    value["password"] = "sensitive".into();
    std::fs::write(&path, serde_json::to_vec(&vec![value]).unwrap()).unwrap();
    let mut s = RemoteService::new(path);
    settle(&mut s);
    assert!(s.profiles.is_empty());
    assert!(s.error.as_ref().is_some_and(|e| !e.contains("sensitive")));
    assert!(s.active.is_none());
}
