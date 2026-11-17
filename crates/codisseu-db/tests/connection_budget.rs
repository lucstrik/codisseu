//! Opt-in, owned loopback SQL fixtures. Never reads personal profiles/credentials.
use codisseu_db::remote::{ConnectionConfig, Engine, RemoteCommand, RemoteService, TableRef};
use sqlx_core::{connection::Connection, query::query, row::Row};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

struct Monitor {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicUsize>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Monitor {
    fn start(port: u16) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicUsize::new(0));
        let (end, high) = (stop.clone(), peak.clone());
        let worker = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let mut connection = sqlx_mysql::MySqlConnection::connect_with(
                    &sqlx_mysql::MySqlConnectOptions::new().host("127.0.0.1").port(port).username("root").password("codisseu_fixture"),
                ).await.unwrap();
                while !end.load(Ordering::SeqCst) {
                    let rows = query("SELECT ID FROM information_schema.PROCESSLIST WHERE USER='codisseu_review'").fetch_all(&mut connection).await.unwrap();
                    high.fetch_max(rows.len(), Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                connection.close().await.unwrap();
            });
        });
        Self {
            stop,
            peak,
            worker: Some(worker),
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn settle(service: &mut RemoteService) {
    let until = Instant::now() + Duration::from_secs(12);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < until, "{}", service.status);
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    settle(service);
    assert!(service.error.is_none(), "{}", service.status);
}

#[test]
#[ignore = "requires owned PostgreSQL fixture on explicit CODISSEU_REVIEW_POSTGRES_PORT"]
fn postgres_server_counts_connections_and_cancel_terminates_exact_backend() {
    let port = std::env::var("CODISSEU_REVIEW_POSTGRES_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut observer = rt
        .block_on(sqlx_postgres::PgConnection::connect_with(
            &sqlx_postgres::PgConnectOptions::new()
                .host("127.0.0.1")
                .port(port)
                .username("postgres")
                .password("codisseu_fixture")
                .database("codisseu")
                .application_name("fixture-observer"),
        ))
        .unwrap();
    rt.block_on(query("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='codisseu_review') THEN CREATE ROLE codisseu_review LOGIN PASSWORD 'codisseu_fixture'; END IF; END $$").execute(&mut observer)).unwrap();
    let mut count = |stage: &str, expected: usize| {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let rows = rt.block_on(query("SELECT pid FROM pg_stat_activity WHERE application_name='Codisseu' ORDER BY pid").fetch_all(&mut observer)).unwrap();
            let ids: Vec<i32> = rows.iter().map(|r| r.get(0)).collect();
            if ids.len() == expected || Instant::now() >= until {
                println!("PostgreSQL {stage}: sessions={}, pids={ids:?}", ids.len());
                assert_eq!(ids.len(), expected);
                return ids;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(dir.path().join("sources.json"));
    settle(&mut service);
    count("startup", 0);
    let config = ConnectionConfig {
        id: "pg-review".into(),
        engine: Engine::Postgres,
        host: "127.0.0.1".into(),
        port,
        username: "codisseu_review".into(),
        database: "codisseu".into(),
        tls: false,
        save_credentials: false,
        timeout_secs: 30,
        idle_secs: 60,
        ..Default::default()
    };
    for cycle in 0..5 {
        send(
            &mut service,
            RemoteCommand::Connect {
                config: config.clone(),
                password: "codisseu_fixture".into(),
                test: false,
            },
        );
        let ids = count("connect", 1);
        for _ in 0..10 {
            send(
                &mut service,
                RemoteCommand::Execute("SELECT pg_backend_pid()".into()),
            );
        }
        assert_eq!(count("10 queries reused backend", 1), ids);
        service.dispatch(RemoteCommand::Execute("SELECT SUM(a.n*b.n) FROM generate_series(1,1000000) a(n) CROSS JOIN generate_series(1,1000000) b(n)".into())).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        service.dispatch(RemoteCommand::Cancel).unwrap();
        settle(&mut service);
        count(&format!("cancel cycle {cycle}"), 0);
    }
    drop(service);
    count("drop", 0);
    rt.block_on(observer.close()).unwrap();
}

#[test]
#[ignore = "requires owned MySQL fixture on explicit CODISSEU_REVIEW_MYSQL_PORT"]
fn mysql_server_counts_connections_through_daily_workflow() {
    let port: u16 = std::env::var("CODISSEU_REVIEW_MYSQL_PORT")
        .unwrap()
        .parse()
        .unwrap();
    let monitor = Monitor::start(port);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut admin = rt
        .block_on(sqlx_mysql::MySqlConnection::connect_with(
            &sqlx_mysql::MySqlConnectOptions::new()
                .host("127.0.0.1")
                .port(port)
                .username("root")
                .password("codisseu_fixture")
                .database("codisseu"),
        ))
        .unwrap();
    for sql in [
        "CREATE USER IF NOT EXISTS 'codisseu_review'@'%' IDENTIFIED BY 'codisseu_fixture'",
        "ALTER USER 'codisseu_review'@'%' IDENTIFIED BY 'codisseu_fixture'",
        "GRANT SELECT ON codisseu.* TO 'codisseu_review'@'%'",
        "CREATE TABLE IF NOT EXISTS connection_fixture(id INT PRIMARY KEY, label VARCHAR(32))",
        "INSERT IGNORE INTO connection_fixture VALUES(1,'one'),(2,'two'),(3,'three')",
    ] {
        rt.block_on(query(sql).execute(&mut admin)).unwrap();
    }
    let mut snapshot = |stage: &str, expected: usize| -> Vec<u64> {
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            let rows = rt.block_on(query("SELECT ID FROM information_schema.PROCESSLIST WHERE USER='codisseu_review' ORDER BY ID").fetch_all(&mut admin)).unwrap();
            let ids: Vec<u64> = rows.iter().map(|r| r.get(0)).collect();
            if ids.len() == expected || Instant::now() >= until {
                println!("{stage}: connections={}, ids={ids:?}", ids.len());
                assert_eq!(ids.len(), expected, "{stage}");
                return ids;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let mut config = ConnectionConfig {
        id: "connection-review".into(),
        name: "Local connection review".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        username: "codisseu_review".into(),
        database: "codisseu".into(),
        tls: false,
        save_credentials: false,
        timeout_secs: 3,
        idle_secs: 5,
        row_limit: 2,
        ..Default::default()
    };
    let path = dir.path().join("sources.json");
    let mut service = RemoteService::new(path.clone());
    settle(&mut service);
    send(&mut service, RemoteCommand::Save(config.clone()));
    snapshot("startup/saved profile", 0);
    let connect_config = config.clone();
    let connect = |test| RemoteCommand::Connect {
        config: connect_config.clone(),
        password: "codisseu_fixture".into(),
        test,
    };
    send(&mut service, connect(false));
    let initial = snapshot("connect", 1);
    let table = TableRef {
        namespace: "codisseu".into(),
        name: "connection_fixture".into(),
    };
    for index in 0..30 {
        for command in [
            RemoteCommand::Execute("SELECT * FROM connection_fixture ORDER BY id".into()),
            RemoteCommand::ListNamespaces,
            RemoteCommand::InspectNamespace("codisseu".into()),
            RemoteCommand::InspectColumns(table.clone()),
            RemoteCommand::LoadTablePage {
                table: table.clone(),
                offset: 2,
            },
        ] {
            send(&mut service, command);
        }
        assert_eq!(
            snapshot(&format!("query/catalog/page cycle {index}"), 1),
            initial
        );
    }
    service
        .dispatch(RemoteCommand::Execute(
            "SELECT missing_column FROM connection_fixture".into(),
        ))
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_some());
    assert_eq!(snapshot("recoverable SQL error", 1), initial);
    for index in 0..10 {
        send(&mut service, connect(false));
        snapshot(&format!("reconnect {index}"), 1);
        send(&mut service, connect(true));
        snapshot(&format!("test connection {index}"), 0);
    }
    send(&mut service, connect(false));
    send(&mut service, RemoteCommand::CreateConsole);
    snapshot("create console", 1);
    let until = Instant::now() + Duration::from_secs(8);
    while !service.suspended {
        service.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    snapshot("idle", 0);
    send(
        &mut service,
        RemoteCommand::ExecuteConsole {
            datasource_id: config.id.clone(),
            sql: "SELECT 1".into(),
        },
    );
    snapshot("resume saved query", 1);
    send(&mut service, RemoteCommand::Disconnect);
    snapshot("disconnect", 0);
    config.timeout_secs = 30;
    send(
        &mut service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
    service
        .dispatch(RemoteCommand::Execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<999) SELECT SUM(a.x*b.x*c.x) FROM n a CROSS JOIN n b CROSS JOIN n c".into()))
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    service.dispatch(RemoteCommand::Cancel).unwrap();
    settle(&mut service);
    println!("cancel status: {}", service.status);
    snapshot("cancel query", 0);
    for index in 0..5 {
        send(
            &mut service,
            RemoteCommand::Connect {
                config: config.clone(),
                password: "codisseu_fixture".into(),
                test: false,
            },
        );
        service.dispatch(RemoteCommand::Execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<999) SELECT SUM(a.x*b.x*c.x) FROM n a CROSS JOIN n b CROSS JOIN n c".into())).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        service.dispatch(RemoteCommand::Cancel).unwrap();
        settle(&mut service);
        snapshot(&format!("repeated cancel {index}"), 0);
    }
    send(&mut service, connect(false));
    service
        .dispatch(RemoteCommand::Execute("SELECT 1".into()))
        .unwrap();
    // Server finished, GUI has deliberately not consumed the queued result.
    std::thread::sleep(Duration::from_millis(200));
    service.dispatch(RemoteCommand::Cancel).unwrap();
    settle(&mut service);
    assert!(service.active.is_none());
    snapshot("late cancel before GUI consumes result", 0);

    // Fail only the synthetic user's auxiliary login. A blocked cleanup must
    // prevent replacement data sessions; restoring credentials permits cleanup.
    let alter_password = |password: &str| {
        rt.block_on(async {
            let mut control = sqlx_mysql::MySqlConnection::connect_with(
                &sqlx_mysql::MySqlConnectOptions::new()
                    .host("127.0.0.1")
                    .port(port)
                    .username("root")
                    .password("codisseu_fixture"),
            )
            .await
            .unwrap();
            query(&format!(
                "ALTER USER 'codisseu_review'@'%' IDENTIFIED BY '{password}'"
            ))
            .execute(&mut control)
            .await
            .unwrap();
            control.close().await.unwrap();
        })
    };
    send(
        &mut service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
    alter_password("fixture_temporarily_changed");
    service.dispatch(RemoteCommand::Execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<999) SELECT SUM(a.x*b.x*c.x) FROM n a CROSS JOIN n b CROSS JOIN n c".into())).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    service.dispatch(RemoteCommand::Cancel).unwrap();
    settle(&mut service);
    assert!(service.status.contains("bloqueada"), "{}", service.status);
    snapshot("failed remote cleanup quarantines original", 1);
    for index in 0..3 {
        service.dispatch(connect(false)).unwrap();
        settle(&mut service);
        assert!(service.error.as_ref().unwrap().contains("bloqueada"));
        snapshot(&format!("blocked reconnect {index}"), 1);
    }
    alter_password("codisseu_fixture");
    send(&mut service, connect(false));
    snapshot("cleanup recovered before reconnect", 1);
    send(&mut service, RemoteCommand::Disconnect);
    snapshot("after cleanup recovery disconnect", 0);
    send(&mut service, connect(false));
    drop(service);
    snapshot("drop service", 0);
    let mut restored = RemoteService::new(path);
    settle(&mut restored);
    snapshot("restore profile and console", 0);
    send(&mut restored, connect(false));
    let mut second = RemoteService::new(dir.path().join("second-instance.json"));
    settle(&mut second);
    send(&mut second, connect(false));
    snapshot("two independent applications", 2);
    drop(restored);
    snapshot("first application closed", 1);
    drop(second);
    snapshot("both applications closed", 0);
    let peak = monitor.peak.load(Ordering::SeqCst);
    println!("Server sampled peak (2ms interval): {peak}; observer uses a separate user");
    assert!(
        peak <= 2,
        "single data connection plus transient cleanup connection"
    );
    drop(monitor);
    rt.block_on(admin.close()).unwrap();
}
