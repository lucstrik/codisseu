//! Opt-in tests against disposable, seeded local instances only.
//! CODISSEU_TEST_REDIS_PORT, CODISSEU_TEST_MEMCACHED_PORT,
//! CODISSEU_TEST_MONGO_PORT are mandatory for the corresponding ignored test.
use codisseu_db::remote::{Cell, ConnectionConfig, Engine, RemoteCommand, RemoteService};
use std::time::{Duration, Instant};

fn fixture_port(variable: &str) -> u16 {
    std::env::var(variable)
        .unwrap_or_else(|_| panic!("{variable} must point to a disposable fixture"))
        .parse()
        .unwrap()
}

fn seed_redis() {
    let port = fixture_port("CODISSEU_TEST_REDIS_PORT");
    let client = redis::Client::open(format!("redis://127.0.0.1:{port}/0")).unwrap();
    let mut connection = client
        .get_connection_with_timeout(Duration::from_secs(3))
        .unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    connection
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    for (key, value) in [
        ("codisseu:name", "Lucas 🦀".as_bytes().to_vec()),
        ("codisseu:space key", b"quoted".to_vec()),
        ("codisseu:large", vec![b'x'; 70_000]),
    ] {
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .query::<()>(&mut connection)
            .unwrap();
    }
    redis::cmd("HSET")
        .arg("codisseu:hash")
        .arg("field")
        .arg("content")
        .query::<usize>(&mut connection)
        .unwrap();
}

fn seed_memcached() {
    use std::io::{Read, Write};
    let port = fixture_port("CODISSEU_TEST_MEMCACHED_PORT");
    let mut connection = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    connection
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    for (key, value) in [
        ("codisseu:name", "Lucas 🦀".as_bytes().to_vec()),
        ("codisseu:binary", vec![0, 255, 13, 10]),
        ("codisseu:large", vec![b'x'; 70_000]),
    ] {
        connection
            .write_all(format!("set {key} 7 0 {}\r\n", value.len()).as_bytes())
            .unwrap();
        connection.write_all(&value).unwrap();
        connection.write_all(b"\r\n").unwrap();
        let mut response = [0; 8];
        connection.read_exact(&mut response).unwrap();
        assert_eq!(&response, b"STORED\r\n");
    }
}

fn seed_mongo() {
    use mongodb::bson::{Document, doc};
    let port = fixture_port("CODISSEU_TEST_MONGO_PORT");
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let client = mongodb::Client::with_uri_str(format!("mongodb://127.0.0.1:{port}/?directConnection=true&serverSelectionTimeoutMS=3000&maxPoolSize=1")).await.unwrap();
        let collection = client.database("codisseu_fixture").collection::<Document>("users");
        for document in [doc! {"_id":1,"name":"Lucas 🦀","active":true,"big":i64::MAX},doc! {"_id":2,"name":"Maria","active":true}] {
            collection.replace_one(doc! {"_id":document.get_i32("_id").unwrap()},document).upsert(true).await.unwrap();
        }
        client.shutdown().await;
    });
}

fn settle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "Timed out: {}", service.status);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn session(engine: Engine, variable: &str) -> (RemoteService, tempfile::TempDir) {
    let port = fixture_port(variable);
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("profiles.json"));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !service.poll() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    service
        .dispatch(RemoteCommand::Connect {
            config: ConnectionConfig {
                id: "disposable-nosql-test".into(),
                name: "Disposable NoSQL fixture".into(),
                engine,
                host: "127.0.0.1".into(),
                port,
                database: if engine == Engine::Mongo {
                    "codisseu_fixture".into()
                } else {
                    "0".into()
                },
                username: String::new(),
                auth_database: String::new(),
                mongo: Default::default(),
                ca_file: String::new(),
                ca_pem: Default::default(),
                tls: false,
                read_only: true,
                timeout_secs: 3,
                idle_secs: 5,
                row_limit: 500,
                save_credentials: false,
            },
            password: String::new(),
            test: false,
        })
        .unwrap();
    settle(&mut service);
    assert!(service.active.is_some(), "{}", service.status);
    (service, directory)
}

fn execute(service: &mut RemoteService, query: &str) {
    service
        .dispatch(RemoteCommand::Execute(query.into()))
        .unwrap();
    settle(service);
    assert!(service.error.is_none(), "{:?}", service.error);
}

#[test]
#[ignore = "requires seeded disposable Redis; set CODISSEU_TEST_REDIS_PORT"]
fn redis_fixture_reads_bounds_rejects_writes_and_suspends_after_idle() {
    seed_redis();
    let (mut service, _directory) = session(Engine::Redis, "CODISSEU_TEST_REDIS_PORT");
    service.dispatch(RemoteCommand::Inspect).unwrap();
    settle(&mut service);
    assert!(service.objects.is_empty());
    execute(
        &mut service,
        "MGET codisseu:name codisseu:missing 'codisseu:space key'",
    );
    let rows = &service.result.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][1], Cell::Text("Lucas 🦀".into()));
    assert_eq!(rows[1][1], Cell::Null);
    assert_eq!(rows[2][1], Cell::Text("quoted".into()));
    execute(&mut service, "GET codisseu:large");
    let output = service.result.as_ref().unwrap();
    assert!(output.truncated);
    assert_eq!(
        match &output.rows[0][1] {
            Cell::Text(s) => s.len(),
            _ => 0,
        },
        65536
    );
    execute(&mut service, "HSCAN codisseu:hash 0 COUNT 10");
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][1],
        Cell::Text("field".into())
    );
    execute(&mut service, "SCAN 0 MATCH codisseu:name COUNT 10");
    service
        .dispatch(RemoteCommand::Execute("FLUSHALL".into()))
        .unwrap();
    settle(&mut service);
    assert!(service.error.as_ref().unwrap().contains("bloqueado"));
    assert!(service.active.is_none());
    // A separate connection proves the rejected mutation did not run.
    let (mut second, _directory2) = session(Engine::Redis, "CODISSEU_TEST_REDIS_PORT");
    execute(&mut second, "GET codisseu:name");
    assert_eq!(
        second.result.as_ref().unwrap().rows[0][1],
        Cell::Text("Lucas 🦀".into())
    );
    std::thread::sleep(Duration::from_secs(6));
    second.poll();
    assert!(second.active.is_some(), "{}", second.status);
    assert!(second.suspended);
    execute(&mut second, "GET codisseu:name");
    assert!(!second.suspended);
    assert_eq!(
        second.result.as_ref().unwrap().rows[0][1],
        Cell::Text("Lucas 🦀".into())
    );
}

#[test]
#[ignore = "requires seeded disposable Memcached; set CODISSEU_TEST_MEMCACHED_PORT"]
fn memcached_fixture_reads_binary_values_stats_and_rejects_large_payloads() {
    seed_memcached();
    let (mut service, _directory) = session(Engine::Memcached, "CODISSEU_TEST_MEMCACHED_PORT");
    execute(
        &mut service,
        "gets codisseu:name codisseu:binary codisseu:missing",
    );
    let rows = &service.result.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1], Cell::Text("Lucas 🦀".into()));
    assert_eq!(rows[1][1], Cell::Binary(vec![0, 255, 13, 10]));
    assert!(matches!(rows[0][3], Cell::Number(_)));
    execute(&mut service, "stats");
    assert!(
        service
            .result
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .any(|row| row[0] == Cell::Text("curr_connections".into()))
    );
    service
        .dispatch(RemoteCommand::Execute("get codisseu:large".into()))
        .unwrap();
    settle(&mut service);
    assert!(service.error.as_ref().unwrap().contains("64 KiB"));
    assert!(service.active.is_none());
}

#[test]
#[ignore = "requires seeded disposable MongoDB; set CODISSEU_TEST_MONGO_PORT"]
fn mongodb_fixture_streams_typed_extjson_limits_and_lazy_collection_schema() {
    seed_mongo();
    let (mut service, _directory) = session(Engine::Mongo, "CODISSEU_TEST_MONGO_PORT");
    assert!(service.objects.is_empty());
    service.dispatch(RemoteCommand::Inspect).unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    assert!(service.objects.iter().any(|object| object.name == "users"));
    execute(
        &mut service,
        r#"{"collection":"users","filter":{"active":true},"sort":{"_id":1},"limit":1}"#,
    );
    let output = service.result.as_ref().unwrap();
    assert_eq!(output.rows.len(), 1);
    assert!(output.truncated);
    let Cell::Json(document) = &output.rows[0][1] else {
        panic!("Expected Extended JSON")
    };
    assert!(document.contains("Lucas"));
    assert!(document.contains("9223372036854775807"));
    execute(
        &mut service,
        r#"{"collection":"users","filter":{"_id":999}}"#,
    );
    assert!(service.result.as_ref().unwrap().rows.is_empty());
    service
        .dispatch(RemoteCommand::Execute(
            r#"{"collection":"users","filter":{"$where":"sleep(1000)"}}"#.into(),
        ))
        .unwrap();
    settle(&mut service);
    assert!(service.error.as_ref().unwrap().contains("JavaScript"));
    assert!(service.active.is_none());
}
