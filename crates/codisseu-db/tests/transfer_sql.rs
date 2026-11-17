//! Real servers only via explicitly supplied disposable loopback fixture ports.
use codisseu_db::{
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand, RemoteService, TableRef},
    transfer::*,
};
use sqlx_core::{connection::Connection, query::query, row::Row};
use std::time::{Duration, Instant};
fn wait(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            service.status,
            service.error
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    service.poll();
}
fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    wait(service);
    assert!(service.error.is_none(), "{}", service.status);
}
fn request(table: TableRef, ids: &[i64]) -> ImportRequest {
    ImportRequest {
        allow_partial: false,
        target: ImportTarget::SqlTable(table),
        data: TransferData {
            columns: vec![
                "id".into(),
                "name".into(),
                "payload".into(),
                "amount".into(),
            ],
            rows: ids
                .iter()
                .map(|id| {
                    vec![
                        Cell::Number(id.to_string()),
                        Cell::Text("Robert'); DROP TABLE codisseu_transfer_fixture;--\\x".into()),
                        Cell::Binary(vec![0, 255, 39, 92]),
                        Cell::Number("12345678901234567890.1234567890".into()),
                    ]
                })
                .collect(),
            source_truncated: false,
        },
        mapping: ["id", "name", "payload", "amount"]
            .iter()
            .enumerate()
            .map(|(i, name)| ImportMapping {
                source_column: i,
                target_column: (*name).into(),
            })
            .collect(),
    }
}
#[test]
#[ignore = "requires explicit disposable PG/MySQL fixture ports"]
fn worker_import_bound_manual_commit_and_atomic_conflict() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (engine, variable) in [
        (Engine::Postgres, "CODISSEU_SQL_PG_PORT"),
        (Engine::Mysql, "CODISSEU_SQL_MY_PORT"),
    ] {
        let port = std::env::var(variable)
            .expect("explicit disposable fixture port")
            .parse()
            .unwrap();
        let pg = engine == Engine::Postgres;
        let cfg = ConnectionConfig {
            id: format!("import-{engine:?}"),
            name: "Disposable import fixture".into(),
            engine,
            host: "127.0.0.1".into(),
            port,
            database: "codisseu".into(),
            username: if pg { "postgres" } else { "root" }.into(),
            tls: false,
            read_only: false,
            timeout_secs: 3,
            idle_secs: 60,
            row_limit: 20,
            ..Default::default()
        };
        let table = TableRef {
            namespace: if pg { "public" } else { "codisseu" }.into(),
            name: "codisseu_transfer_fixture".into(),
        };
        rt.block_on(async{if pg{let options=sqlx_postgres::PgConnectOptions::new().host("127.0.0.1").port(port).username("postgres").password("codisseu_fixture").database("codisseu");let mut c=sqlx_postgres::PgConnection::connect_with(&options).await.unwrap();for sql in ["DROP TABLE IF EXISTS codisseu_transfer_fixture","CREATE TABLE codisseu_transfer_fixture(id BIGINT PRIMARY KEY,name TEXT,payload BYTEA,amount DECIMAL(30,10))","INSERT INTO codisseu_transfer_fixture VALUES(1,'existing',NULL,0)"]{query(sql).execute(&mut c).await.unwrap();}c.close().await.unwrap();}else{let options=sqlx_mysql::MySqlConnectOptions::new().host("127.0.0.1").port(port).username("root").password("codisseu_fixture").database("codisseu");let mut c=sqlx_mysql::MySqlConnection::connect_with(&options).await.unwrap();for sql in ["DROP TABLE IF EXISTS codisseu_transfer_fixture","CREATE TABLE codisseu_transfer_fixture(id BIGINT PRIMARY KEY,name TEXT,payload BLOB,amount DECIMAL(30,10)) ENGINE=InnoDB","INSERT INTO codisseu_transfer_fixture VALUES(1,'existing',NULL,0)"]{query(sql).execute(&mut c).await.unwrap();}c.close().await.unwrap();}});
        let dir = tempfile::tempdir().unwrap();
        let mut service = RemoteService::new(dir.path().join("profiles.json"));
        wait(&mut service);
        send(
            &mut service,
            RemoteCommand::Connect {
                config: cfg.clone(),
                password: "codisseu_fixture".into(),
                test: false,
            },
        );
        let generation = service.session_generation;
        assert!(
            service
                .dispatch(RemoteCommand::PreviewImport(TransferRequest {
                    datasource_id: cfg.id.clone(),
                    session_generation: generation + 1,
                    payload: request(table.clone(), &[2])
                }))
                .is_err()
        );
        send(
            &mut service,
            RemoteCommand::PreviewImport(TransferRequest {
                datasource_id: cfg.id.clone(),
                session_generation: generation,
                payload: request(table.clone(), &[2]),
            }),
        );
        assert_eq!(service.import_preview.as_ref().unwrap().rows, 1);
        let token = service.import_preview.as_ref().unwrap().preview_id;
        send(
            &mut service,
            RemoteCommand::ApplyImport(TransferRequest {
                datasource_id: cfg.id.clone(),
                session_generation: generation,
                payload: ImportApproval { preview_id: token },
            }),
        );
        assert_eq!(service.import_report.as_ref().unwrap().applied, 1);
        assert!(service.transaction);
        let outside = rt.block_on(async {
            if pg {
                let options = sqlx_postgres::PgConnectOptions::new()
                    .host("127.0.0.1")
                    .port(port)
                    .username("postgres")
                    .password("codisseu_fixture")
                    .database("codisseu");
                let mut c = sqlx_postgres::PgConnection::connect_with(&options)
                    .await
                    .unwrap();
                let count: i64 = query("SELECT count(*) FROM codisseu_transfer_fixture WHERE id=2")
                    .fetch_one(&mut c)
                    .await
                    .unwrap()
                    .get(0);
                c.close().await.unwrap();
                count
            } else {
                let options = sqlx_mysql::MySqlConnectOptions::new()
                    .host("127.0.0.1")
                    .port(port)
                    .username("root")
                    .password("codisseu_fixture")
                    .database("codisseu");
                let mut c = sqlx_mysql::MySqlConnection::connect_with(&options)
                    .await
                    .unwrap();
                let count: i64 = query("SELECT count(*) FROM codisseu_transfer_fixture WHERE id=2")
                    .fetch_one(&mut c)
                    .await
                    .unwrap()
                    .get(0);
                c.close().await.unwrap();
                count
            }
        });
        assert_eq!(outside, 0, "import leaked before Commit");
        // A duplicate after an earlier row rolls back only this import batch, preserving prior pending import.
        send(
            &mut service,
            RemoteCommand::PreviewImport(TransferRequest {
                datasource_id: cfg.id.clone(),
                session_generation: generation,
                payload: request(table.clone(), &[3, 1]),
            }),
        );
        let token = service.import_preview.as_ref().unwrap().preview_id;
        send(
            &mut service,
            RemoteCommand::ApplyImport(TransferRequest {
                datasource_id: cfg.id.clone(),
                session_generation: generation,
                payload: ImportApproval { preview_id: token },
            }),
        );
        assert_eq!(service.import_report.as_ref().unwrap().applied, 0);
        assert!(service.transaction);
        send(&mut service, RemoteCommand::Commit);
        assert!(!service.transaction);
        send(
            &mut service,
            RemoteCommand::Execute(
                "SELECT id,name,payload,amount FROM codisseu_transfer_fixture ORDER BY id".into(),
            ),
        );
        let result = service.result.as_ref().unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[1][0], Cell::Number("2".into()));
        assert_eq!(
            result.rows[1][3],
            Cell::Number("12345678901234567890.1234567890".into())
        );
        assert!(matches!(&result.rows[1][2],Cell::Binary(v) if *v==vec![0,255,39,92]));
        send(&mut service, RemoteCommand::Rollback);
        let mut readonly = cfg.clone();
        readonly.read_only = true;
        send(&mut service, RemoteCommand::Save(readonly));
        assert!(
            service
                .dispatch(RemoteCommand::ApplyImport(TransferRequest {
                    datasource_id: cfg.id.clone(),
                    session_generation: generation,
                    payload: ImportApproval { preview_id: token }
                }))
                .is_err()
        );
        send(&mut service, RemoteCommand::Disconnect);
    }
}
