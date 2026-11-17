//! Real single-worker transaction lifecycle. Explicit disposable loopback fixture only.
use codisseu_db::remote::{
    Cell, CellEdit, ConnectionConfig, EditBatch, Engine, RemoteCommand, RemoteService, TableRef,
};
use sqlx_core::{connection::Connection, query::query, row::Row};
use std::time::{Duration, Instant};

fn poll_until_idle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "worker stuck busy: {} / {:?}",
            service.status,
            service.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    poll_until_idle(service);
    assert!(service.error.is_none(), "{}", service.status);
}

#[test]
#[ignore = "requires explicit CODISSEU_SQL_PG_PORT for disposable PostgreSQL fixture"]
fn worker_manual_transactions_finish_and_keep_one_session_usable() {
    let port = std::env::var("CODISSEU_SQL_PG_PORT")
        .expect("disposable loopback PostgreSQL fixture port")
        .parse()
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let options = sqlx_postgres::PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .database("codisseu")
        .username("postgres")
        .password("codisseu_fixture");
    let mut observer = rt
        .block_on(sqlx_postgres::PgConnection::connect_with(&options))
        .unwrap();
    for sql in [
        "DROP TABLE IF EXISTS codisseu_service_fixture",
        "CREATE TABLE codisseu_service_fixture(id BIGINT PRIMARY KEY,name TEXT)",
        "INSERT INTO codisseu_service_fixture VALUES(1,'original')",
    ] {
        rt.block_on(query(sql).execute(&mut observer)).unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let profiles = directory.path().join("datasources.json");
    let config = ConnectionConfig {
        id: "service-fixture".into(),
        name: "Disposable worker fixture".into(),
        engine: Engine::Postgres,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: "postgres".into(),
        tls: false,
        read_only: false,
        timeout_secs: 3,
        idle_secs: 60,
        row_limit: 5,
        ..ConnectionConfig::default()
    };
    std::fs::write(
        &profiles,
        serde_json::to_vec(&vec![config.clone()]).unwrap(),
    )
    .unwrap();
    let mut service = RemoteService::new(profiles.clone());
    poll_until_idle(&mut service);
    assert_eq!(service.profiles.len(), 1);
    assert!(
        service.active.is_none(),
        "loading profiles must not auto-connect"
    );
    send(
        &mut service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
    let table = TableRef {
        namespace: "public".into(),
        name: "codisseu_service_fixture".into(),
    };
    send(&mut service, RemoteCommand::LoadTable(table.clone()));
    let (origin, columns) = service.origin.clone().unwrap();
    assert_eq!(origin, table);
    let batch = EditBatch {
        table: table.clone(),
        columns,
        edits: vec![CellEdit {
            row: service.result.as_ref().unwrap().rows[0].clone(),
            column: "name".into(),
            value: Cell::Text("committed".into()),
        }],
    };
    send(&mut service, RemoteCommand::Apply(batch));
    assert!(service.transaction);
    let outside: String = rt
        .block_on(
            query("SELECT name FROM codisseu_service_fixture WHERE id=1").fetch_one(&mut observer),
        )
        .unwrap()
        .get(0);
    assert_eq!(
        outside, "original",
        "uncommitted changes leaked to independent session"
    );
    for test in [false, true] {
        assert!(
            service
                .dispatch(RemoteCommand::Connect {
                    config: config.clone(),
                    password: "codisseu_fixture".into(),
                    test
                })
                .is_err(),
            "pending transaction must block connect/test"
        );
        assert!(service.active.is_some());
        assert!(!service.is_busy());
    }
    send(&mut service, RemoteCommand::Commit);
    assert!(!service.transaction);
    let outside: String = rt
        .block_on(
            query("SELECT name FROM codisseu_service_fixture WHERE id=1").fetch_one(&mut observer),
        )
        .unwrap()
        .get(0);
    assert_eq!(outside, "committed");
    send(
        &mut service,
        RemoteCommand::ExecuteBound {
            sql: "SELECT name FROM codisseu_service_fixture WHERE id=1".into(),
            datasource_id: config.id.clone(),
        },
    );
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Text("committed".into())
    );
    assert!(
        !service.transaction,
        "standalone SELECT must release its snapshot after a committed edit"
    );
    send(&mut service, RemoteCommand::Rollback);
    assert!(!service.transaction);
    // Reject the wrong console target before sending anything or invalidating current data.
    let revision = service.revision;
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteBound {
                sql: "SELECT 'wrong datasource'".into(),
                datasource_id: "another-datasource".into()
            })
            .is_err()
    );
    assert_eq!(service.revision, revision);
    assert!(service.active.is_some());
    assert!(!service.is_busy());
    send(&mut service, RemoteCommand::LoadTable(table.clone()));
    let (_, columns) = service.origin.clone().unwrap();
    let row = service.result.as_ref().unwrap().rows[0].clone();
    send(
        &mut service,
        RemoteCommand::Apply(EditBatch {
            table,
            columns,
            edits: vec![CellEdit {
                row,
                column: "name".into(),
                value: Cell::Text("rolled back".into()),
            }],
        }),
    );
    send(&mut service, RemoteCommand::Rollback);
    assert!(!service.transaction);
    send(
        &mut service,
        RemoteCommand::ExecuteBound {
            sql: "SELECT name FROM codisseu_service_fixture WHERE id=1".into(),
            datasource_id: config.id.clone(),
        },
    );
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Text("committed".into())
    );
    let outside: String = rt
        .block_on(
            query("SELECT name FROM codisseu_service_fixture WHERE id=1").fetch_one(&mut observer),
        )
        .unwrap()
        .get(0);
    assert_eq!(outside, "committed");
    send(&mut service, RemoteCommand::Rollback);
    send(
        &mut service,
        RemoteCommand::LoadTable(TableRef {
            namespace: "public".into(),
            name: "codisseu_service_fixture".into(),
        }),
    );
    let (table, columns) = service.origin.clone().unwrap();
    let row = service.result.as_ref().unwrap().rows[0].clone();
    let mut safer = config.clone();
    safer.read_only = true;
    send(&mut service, RemoteCommand::Save(safer));
    assert!(
        service
            .dispatch(RemoteCommand::Apply(EditBatch {
                table,
                columns,
                edits: vec![CellEdit {
                    row,
                    column: "name".into(),
                    value: Cell::Text("must be blocked".into())
                }]
            }))
            .is_err(),
        "saved read-only preference must block existing writable session"
    );
    assert!(!service.is_busy());
    assert!(service.active.is_some());
    send(&mut service, RemoteCommand::Rollback);
    let persisted = std::fs::read_to_string(&profiles).unwrap();
    assert!(!persisted.contains("password"));
    assert!(!persisted.contains("codisseu_fixture"));
    send(&mut service, RemoteCommand::Disconnect);
    assert!(service.active.is_none());
    rt.block_on(query("DROP TABLE codisseu_service_fixture").execute(&mut observer))
        .unwrap();
    rt.block_on(observer.close()).unwrap();
}
