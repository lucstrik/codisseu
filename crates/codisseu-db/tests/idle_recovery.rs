//! Idle socket recovery against owned loopback fixtures. No customer profiles.
use codisseu_db::remote::{
    Cell, CellEdit, ConnectionConfig, EditBatch, Engine, RemoteCommand, RemoteService, TableRef,
};
use codisseu_db::transfer::TransferRequest;
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

fn settle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}

fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    settle(service);
    assert!(service.error.is_none(), "{}", service.status);
}

fn wait_for(service: &mut RemoteService, predicate: impl Fn(&RemoteService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate(service) {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        std::thread::sleep(Duration::from_millis(5));
    }
}

struct MemcachedFixture {
    port: u16,
    accepted: Arc<AtomicUsize>,
    handshakes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl MemcachedFixture {
    fn new(block_reconnect: bool) -> Self {
        Self::with_delay(block_reconnect, Duration::ZERO)
    }

    fn with_delay(block_reconnect: bool, response_delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let handshakes = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (server_accepted, server_handshakes, server_stop) =
            (accepted.clone(), handshakes.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            let mut clients = Vec::new();
            while !server_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let identity = server_accepted.fetch_add(1, Ordering::SeqCst) + 1;
                        let (handshakes, stop) = (server_handshakes.clone(), server_stop.clone());
                        clients.push(std::thread::spawn(move || {
                            memcached_client(
                                socket,
                                identity,
                                block_reconnect,
                                response_delay,
                                handshakes,
                                stop,
                            );
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            port,
            accepted,
            handshakes,
            stop,
            worker: Some(worker),
        }
    }

    fn config(&self) -> ConnectionConfig {
        ConnectionConfig {
            id: "idle-fixture".into(),
            name: "Disposable idle fixture".into(),
            engine: Engine::Memcached,
            host: "127.0.0.1".into(),
            port: self.port,
            tls: false,
            timeout_secs: 3,
            idle_secs: 5,
            save_credentials: false,
            ..Default::default()
        }
    }
}

impl Drop for MemcachedFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn memcached_client(
    socket: TcpStream,
    identity: usize,
    block_reconnect: bool,
    response_delay: Duration,
    handshakes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
) {
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut stream = BufReader::new(socket);
    while !stop.load(Ordering::SeqCst) {
        let mut line = String::new();
        match stream.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line == "version\r\n" {
                    handshakes.fetch_add(1, Ordering::SeqCst);
                    if block_reconnect && identity > 1 {
                        continue;
                    }
                    if stream
                        .get_mut()
                        .write_all(b"VERSION 1.6.fixture\r\n")
                        .is_err()
                    {
                        break;
                    }
                } else if line == "get identity\r\n" {
                    std::thread::sleep(response_delay);
                    let value = identity.to_string();
                    let response =
                        format!("VALUE identity 0 {}\r\n{value}\r\nEND\r\n", value.len());
                    if stream.get_mut().write_all(response.as_bytes()).is_err() {
                        break;
                    }
                } else {
                    panic!("Unexpected fixture command: {line:?}");
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
}

fn connect(service: &mut RemoteService, config: &ConnectionConfig, password: &str) {
    send(
        service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: password.into(),
            test: false,
        },
    );
}

fn execute(service: &mut RemoteService, config: &ConnectionConfig, sql: &str) {
    send(
        service,
        RemoteCommand::ExecuteBound {
            sql: sql.into(),
            datasource_id: config.id.clone(),
        },
    );
}

#[test]
fn idle_releases_socket_but_keeps_source_result_and_resumes_only_on_next_read() {
    let fixture = MemcachedFixture::new(false);
    let config = fixture.config();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sources.json");
    let mut service = RemoteService::new(path.clone());
    settle(&mut service);
    connect(&mut service, &config, "");
    execute(&mut service, &config, "get identity");
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][1],
        Cell::Text("1".into())
    );
    let (revision, generation, logical) = (
        service.revision,
        service.session_generation,
        service.connection_generation,
    );
    // Cached catalog data belongs to the logical source, not the released socket.
    service.namespaces.push("cached namespace".into());
    service.namespaces_loaded = true;
    wait_for(&mut service, |s| s.suspended);
    assert_eq!(service.active.as_ref().unwrap().id, config.id);
    assert_eq!(service.revision, revision);
    assert!(service.result.is_some());
    assert!(service.namespaces_loaded);
    assert_eq!(service.namespaces, ["cached namespace"]);
    assert_eq!(service.session_generation, generation + 1);
    assert_eq!(service.connection_generation, logical);
    assert_eq!(
        fixture.accepted.load(Ordering::SeqCst),
        1,
        "no background reconnect"
    );
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteBound {
                sql: "get identity".into(),
                datasource_id: "another-source".into(),
            })
            .is_err()
    );
    assert!(
        service
            .dispatch(RemoteCommand::Explain(TransferRequest {
                datasource_id: config.id.clone(),
                session_generation: generation,
                payload: "SELECT 1".into(),
            }))
            .is_err(),
        "stale physical-session requests must be rejected"
    );
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1);
    execute(&mut service, &config, "get identity");
    assert!(!service.suspended);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][1],
        Cell::Text("2".into())
    );
    assert_eq!(service.session_generation, generation + 1);
    assert_eq!(service.connection_generation, logical);
    execute(&mut service, &config, "get identity");
    assert_eq!(
        fixture.accepted.load(Ordering::SeqCst),
        2,
        "reuse resumed socket"
    );
    send(&mut service, RemoteCommand::Disconnect);
    assert!(service.active.is_none());
    assert!(!service.suspended);
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteBound {
                sql: "get identity".into(),
                datasource_id: config.id.clone(),
            })
            .is_err()
    );
    let mut restored = RemoteService::new(path);
    settle(&mut restored);
    assert!(
        restored.active.is_none(),
        "startup never acquires an idle connection"
    );
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 2);
}

#[test]
fn cancelling_an_idle_reconnect_closes_its_socket_and_removes_recovery_target() {
    let fixture = MemcachedFixture::new(true);
    let config = fixture.config();
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("sources.json"));
    settle(&mut service);
    connect(&mut service, &config, "");
    wait_for(&mut service, |s| s.suspended);
    service
        .dispatch(RemoteCommand::ExecuteBound {
            sql: "get identity".into(),
            datasource_id: config.id,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.handshakes.load(Ordering::SeqCst) < 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    send_cancel(&mut service);
    assert!(service.active.is_none());
    assert!(!service.suspended);
    assert!(service.error.as_ref().unwrap().contains("cancelada"));
    service
        .dispatch(RemoteCommand::Execute("get identity".into()))
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_some());
    assert_eq!(
        fixture.accepted.load(Ordering::SeqCst),
        2,
        "no cancelled read replay"
    );
}

fn send_cancel(service: &mut RemoteService) {
    service.dispatch(RemoteCommand::Cancel).unwrap();
    settle(service);
}

#[test]
fn cancelling_while_suspended_drops_the_logical_connection_without_reopening() {
    let fixture = MemcachedFixture::new(false);
    let config = fixture.config();
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("sources.json"));
    settle(&mut service);
    connect(&mut service, &config, "");
    wait_for(&mut service, |s| s.suspended);
    send_cancel(&mut service);
    assert!(service.active.is_none());
    assert!(!service.suspended);
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1);
}

#[test]
fn worker_rejects_a_stale_request_even_before_the_ui_polls_the_idle_event() {
    let fixture = MemcachedFixture::new(false);
    let config = fixture.config();
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("sources.json"));
    settle(&mut service);
    connect(&mut service, &config, "");
    let generation = service.session_generation;
    // Deliberately do not poll: the GUI still has the old generation, while
    // the worker has released the socket and queued its suspension event.
    std::thread::sleep(Duration::from_millis(5400));
    assert!(!service.suspended);
    service
        .dispatch(RemoteCommand::Explain(TransferRequest {
            datasource_id: config.id.clone(),
            session_generation: generation,
            payload: "SELECT 1".into(),
        }))
        .unwrap();
    settle(&mut service);
    assert!(service.suspended);
    assert_eq!(service.active.as_ref().unwrap().id, config.id);
    assert!(
        service
            .error
            .as_ref()
            .unwrap()
            .contains("Sessão/datasource mudou")
    );
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1);
    execute(&mut service, &config, "get identity");
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 2);
}

#[test]
fn time_spent_executing_a_read_does_not_expire_a_just_completed_session() {
    let fixture = MemcachedFixture::with_delay(false, Duration::from_millis(5200));
    let mut config = fixture.config();
    config.timeout_secs = 8;
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("sources.json"));
    settle(&mut service);
    connect(&mut service, &config, "");
    execute(&mut service, &config, "get identity");
    std::thread::sleep(Duration::from_millis(350));
    service.poll();
    assert!(
        !service.suspended,
        "server execution time is not inactivity"
    );
    assert!(service.active.is_some());
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1);
}

#[test]
#[ignore = "requires owned disposable MySQL via CODISSEU_IDLE_MY_PORT"]
fn mysql_read_recovers_with_same_credentials_and_transaction_idle_is_explicit_rollback() {
    let port = std::env::var("CODISSEU_IDLE_MY_PORT")
        .expect("owned loopback fixture port")
        .parse()
        .unwrap();
    let config = ConnectionConfig {
        id: "idle-mysql-fixture".into(),
        name: "Disposable MySQL idle".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: "root".into(),
        tls: false,
        idle_secs: 5,
        timeout_secs: 3,
        save_credentials: false,
        ..Default::default()
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sources.json");
    let mut service = RemoteService::new(path.clone());
    settle(&mut service);
    connect(&mut service, &config, "codisseu_fixture");
    execute(&mut service, &config, "SELECT CONNECTION_ID()");
    let original = service.result.as_ref().unwrap().rows[0][0].clone();
    wait_for(&mut service, |s| s.suspended);
    execute(&mut service, &config, "SELECT CONNECTION_ID()");
    assert_ne!(service.result.as_ref().unwrap().rows[0][0], original);
    assert_eq!(service.active.as_ref().unwrap().id, config.id);
    assert!(!service.transaction);
    let persisted = std::fs::read_to_string(path).unwrap();
    assert!(!persisted.contains("codisseu_fixture"));
    let mut writable = config.clone();
    writable.read_only = false;
    use sqlx_core::{connection::Connection, query::query, row::Row};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let options = sqlx_mysql::MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .database("codisseu")
        .username("root")
        .password("codisseu_fixture")
        .ssl_mode(sqlx_mysql::MySqlSslMode::Disabled);
    let mut observer = runtime
        .block_on(sqlx_mysql::MySqlConnection::connect_with(&options))
        .unwrap();
    let table = TableRef {
        namespace: "codisseu".into(),
        name: format!("codisseu_idle_{}", std::process::id()),
    };
    for sql in [
        format!(
            "CREATE TABLE `{}` (id BIGINT PRIMARY KEY, name VARCHAR(64)) ENGINE=InnoDB",
            table.name
        ),
        format!("INSERT INTO `{}` VALUES (1, 'original')", table.name),
    ] {
        runtime
            .block_on(query(&sql).execute(&mut observer))
            .unwrap();
    }
    connect(&mut service, &writable, "codisseu_fixture");
    execute(&mut service, &writable, "SELECT 1");
    assert!(
        !service.transaction,
        "a standalone read is not an uncommitted edit"
    );
    send(&mut service, RemoteCommand::LoadTable(table.clone()));
    let columns = service.origin.as_ref().unwrap().1.clone();
    let original_row = service.result.as_ref().unwrap().rows[0].clone();
    send(
        &mut service,
        RemoteCommand::Apply(EditBatch {
            table: table.clone(),
            columns,
            edits: vec![CellEdit {
                row: original_row,
                column: "name".into(),
                value: Cell::Text("pending".into()),
            }],
        }),
    );
    assert!(service.transaction);
    wait_for(&mut service, |s| s.active.is_none());
    assert!(!service.suspended);
    assert!(!service.transaction);
    assert!(service.status.contains("inatividade"));
    assert!(service.status.contains("revertida"));
    let row = runtime
        .block_on(
            query(&format!("SELECT name FROM `{}` WHERE id=1", table.name))
                .fetch_one(&mut observer),
        )
        .unwrap();
    assert_eq!(
        row.get::<String, _>(0),
        "original",
        "idle closure must roll back the pending edit"
    );
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteBound {
                sql: "SELECT 1".into(),
                datasource_id: writable.id,
            })
            .is_err(),
        "a discarded manual transaction cannot be resumed"
    );
    runtime
        .block_on(query(&format!("DROP TABLE `{}`", table.name)).execute(&mut observer))
        .unwrap();
    runtime.block_on(observer.close()).unwrap();
}
