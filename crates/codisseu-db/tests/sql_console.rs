//! Real native console SQL on explicitly supplied, disposable loopback fixtures.
use codisseu_db::{
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand, RemoteService},
    remote_sql::SqlSession,
};
use std::time::{Duration, Instant};

fn config(engine: Engine, key: &str, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        id: format!("console-{key}"),
        name: format!("Disposable console {key}"),
        engine,
        host: "127.0.0.1".into(),
        port: std::env::var(format!("CODISSEU_CONSOLE_{key}_PORT"))
            .unwrap()
            .parse()
            .unwrap(),
        database: "codisseu".into(),
        username: if engine == Engine::Postgres {
            "postgres"
        } else {
            "root"
        }
        .into(),
        tls: false,
        read_only,
        save_credentials: false,
        timeout_secs: 4,
        idle_secs: 60,
        row_limit: 5,
        ..Default::default()
    }
}
async fn exercise(engine: Engine, key: &str) {
    let cfg = config(engine, key, false);
    let mut s = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    s.execute("DROP TABLE IF EXISTS console_fixture")
        .await
        .unwrap();
    s.execute("CREATE TABLE console_fixture(id INTEGER PRIMARY KEY, name VARCHAR(80))")
        .await
        .unwrap();
    assert!(!s.transaction_active());
    let insert = s
        .execute("INSERT INTO console_fixture VALUES(1,'before'),(2,'second')")
        .await
        .unwrap();
    assert_eq!(insert.affected_rows, 2);
    assert!(insert.columns.is_empty());
    assert!(s.transaction_active());
    assert!(s.set_read_only(true).await.is_err());
    s.execute("COMMIT").await.unwrap();
    assert!(!s.transaction_active());
    for (sql, expected) in [
        ("UPDATE console_fixture SET name='pending' WHERE id=1", 1),
        ("DELETE FROM console_fixture WHERE id=2", 1),
    ] {
        assert_eq!(s.execute(sql).await.unwrap().affected_rows, expected);
    }
    // An error must preserve earlier DML, not discard the entire transaction.
    assert!(
        s.execute("INSERT INTO console_fixture VALUES(1,'duplicate')")
            .await
            .is_err()
    );
    assert!(s.transaction_active());
    assert_eq!(
        s.execute("SELECT name FROM console_fixture WHERE id=1")
            .await
            .unwrap()
            .rows[0][0],
        Cell::Text("pending".into())
    );
    s.execute("ROLLBACK").await.unwrap();
    assert!(!s.transaction_active());
    assert_eq!(
        s.execute("SELECT name FROM console_fixture WHERE id=1")
            .await
            .unwrap()
            .rows[0][0],
        Cell::Text("before".into())
    );
    assert!(!s.transaction_active());
    if engine != Engine::Postgres {
        s.execute("CREATE DATABASE IF NOT EXISTS console_transport")
            .await
            .unwrap();
        s.execute("CREATE TABLE IF NOT EXISTS console_transport.transports(id INTEGER PRIMARY KEY,descricao VARCHAR(80))").await.unwrap();
    }
    s.set_read_only(true).await.unwrap();
    for sql in [
        "INSERT INTO console_fixture VALUES(3,'blocked')",
        "UPDATE console_fixture SET name='blocked'",
        "DELETE FROM console_fixture",
        "CREATE TABLE console_blocked(id int)",
        "DROP TABLE console_fixture",
        "SET SESSION TRANSACTION READ WRITE",
        "BEGIN",
        "EXPLAIN ANALYZE UPDATE console_fixture SET name='blocked'",
    ] {
        let error = s.execute(sql).await.unwrap_err();
        assert!(error.contains("Read-only"), "{key}: {sql}: {error}");
        assert!(s.is_connected());
        assert!(!s.transaction_active());
    }
    let inspections: Vec<&str> = if engine == Engine::Postgres {
        vec![
            "SHOW server_version",
            "SHOW ALL",
            "EXPLAIN SELECT * FROM console_fixture",
            "EXPLAIN (FORMAT JSON) SELECT * FROM console_fixture",
        ]
    } else {
        vec![
            "describe console_transport.transports;",
            "DESCRIBE codisseu.console_fixture;",
            "DESC codisseu.console_fixture",
            "SHOW TABLES FROM codisseu",
            "SHOW FULL COLUMNS FROM console_fixture",
            "SHOW CREATE TABLE console_fixture",
            "SHOW INDEX FROM console_fixture",
            "SHOW VARIABLES LIKE 'version'",
            "SHOW ENGINES",
            "SHOW FULL PROCESSLIST",
            "SHOW GRANTS FOR CURRENT_USER",
            "EXPLAIN SELECT * FROM console_fixture",
            "EXPLAIN FORMAT=JSON SELECT * FROM console_fixture",
        ]
    };
    for sql in inspections {
        let result = s
            .execute(sql)
            .await
            .unwrap_or_else(|e| panic!("{key}: {sql}: {e}"));
        assert!(!result.columns.is_empty(), "{sql}");
        assert!(!result.rows.is_empty(), "{sql}");
        if sql.starts_with("describe console_") {
            assert_eq!(result.rows[0][0], Cell::Text("id".into()));
            assert!(
                matches!(&result.rows[0][1], Cell::Text(kind) if kind.starts_with("int")),
                "{result:?}"
            );
            assert_eq!(result.rows[0][3], Cell::Text("PRI".into()));
        }
        assert!(!s.transaction_active());
        assert!(
            s.certify_result(sql, &mut result.clone())
                .await
                .unwrap()
                .is_none()
        );
    }
    s.set_read_only(false).await.unwrap();
    if engine != Engine::Postgres {
        assert!(
            s.execute("DESCRIBE codisseu.console_missing")
                .await
                .is_err()
        );
        let errors = s.execute("SHOW ERRORS").await.unwrap();
        assert!(
            !errors.rows.is_empty(),
            "diagnostics must survive controls/prepare"
        );
    }
    s.set_read_only(false).await.unwrap();
    s.execute("BEGIN").await.unwrap();
    assert!(s.transaction_active());
    s.execute("UPDATE console_fixture SET name='explicit' WHERE id=1")
        .await
        .unwrap();
    s.execute("COMMIT").await.unwrap();
    assert!(!s.transaction_active());
    if engine == Engine::Postgres {
        let returning = s
            .execute("UPDATE console_fixture SET name='returning' RETURNING id,name")
            .await
            .unwrap();
        assert_eq!(returning.rows.len(), 2);
        assert_eq!(returning.affected_rows, 2);
        s.rollback().await.unwrap();
        s.execute("CREATE TEMP TABLE console_many AS SELECT generate_series(1,20) AS id")
            .await
            .unwrap();
        let result = s
            .execute("UPDATE console_many SET id=id+1 RETURNING id")
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 5);
        assert!(result.truncated);
        assert_eq!(result.affected_rows, 20);
        s.rollback().await.unwrap();
    } else {
        s.execute("UPDATE console_fixture SET name='implicit' WHERE id=1")
            .await
            .unwrap();
        s.execute("ALTER TABLE console_fixture ADD COLUMN note VARCHAR(40)")
            .await
            .unwrap();
        assert!(
            !s.transaction_active(),
            "native MySQL DDL commits implicitly"
        );
        s.rollback().await.unwrap();
        assert_eq!(
            s.execute("SELECT name FROM console_fixture WHERE id=1")
                .await
                .unwrap()
                .rows[0][0],
            Cell::Text("implicit".into())
        );
        s.execute("SET TRANSACTION READ ONLY").await.unwrap();
        let denial = s
            .execute("UPDATE console_fixture SET name='forbidden' WHERE id=1")
            .await
            .unwrap_err();
        assert!(
            denial.contains("25006") || denial.contains("25000"),
            "{denial}"
        );
        s.rollback().await.unwrap();
        s.execute("SET autocommit=0").await.unwrap();
        assert!(s.transaction_active());
        s.execute("UPDATE console_fixture SET name='manual-mode' WHERE id=1")
            .await
            .unwrap();
        s.commit().await.unwrap();
        assert!(s.transaction_active());
        s.execute("SET autocommit=1").await.unwrap();
        assert!(!s.transaction_active());
    }
    assert!(
        s.execute("SELECT 1; DROP TABLE console_fixture")
            .await
            .is_err()
    );
    s.execute("DROP TABLE console_fixture").await.unwrap();
    s.close().await.unwrap();
}
fn settle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
fn saved_permission(engine: Engine, key: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(dir.path().join("connections.json"));
    settle(&mut service);
    let mut cfg = config(engine, key, true);
    service
        .dispatch(RemoteCommand::Connect {
            config: cfg.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    let generation = service.connection_generation;
    service
        .dispatch(RemoteCommand::Execute(
            "CREATE TABLE console_saved(id int)".into(),
        ))
        .unwrap();
    settle(&mut service);
    assert!(
        service
            .error
            .as_ref()
            .is_some_and(|e| e.contains("Read-only"))
    );
    cfg.read_only = false;
    service
        .dispatch(RemoteCommand::SaveRemembered {
            config: cfg.clone(),
            password: String::new(),
        })
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    assert!(!service.active.as_ref().unwrap().read_only);
    assert_eq!(service.connection_generation, generation);
    for sql in [
        "DROP TABLE IF EXISTS console_saved",
        "CREATE TABLE console_saved(id int)",
        "INSERT INTO console_saved VALUES(1)",
    ] {
        service
            .dispatch(RemoteCommand::Execute(sql.into()))
            .unwrap();
        settle(&mut service);
        assert!(service.error.is_none(), "{:?}", service.error);
    }
    assert!(service.transaction);
    cfg.read_only = true;
    assert!(service.dispatch(RemoteCommand::Save(cfg.clone())).is_err());
    service.dispatch(RemoteCommand::Rollback).unwrap();
    settle(&mut service);
    service.dispatch(RemoteCommand::Save(cfg)).unwrap();
    settle(&mut service);
    assert!(service.active.as_ref().unwrap().read_only);
    assert_eq!(service.connection_generation, generation);
    service
        .dispatch(RemoteCommand::Execute("DELETE FROM console_saved".into()))
        .unwrap();
    settle(&mut service);
    assert!(
        service
            .error
            .as_ref()
            .is_some_and(|e| e.contains("Read-only"))
    );
    service.dispatch(RemoteCommand::Disconnect).unwrap();
    settle(&mut service);
}
#[test]
#[ignore = "requires owned loopback PostgreSQL/MySQL/MariaDB fixtures via CODISSEU_CONSOLE_{PG,MY,MA}_PORT"]
fn native_console_all_three_servers_and_saved_permission() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (engine, key) in [
        (Engine::Postgres, "PG"),
        (Engine::Mysql, "MY"),
        (Engine::Mysql, "MA"),
    ] {
        rt.block_on(exercise(engine, key));
        saved_permission(engine, key);
    }
}
