//! Explicit disposable loopback MySQL fixture only; never normal user databases.
use codisseu_db::{
    query_export::{CsvExportOptions, QueryExportProgress, QueryExportRequest},
    remote::{ConnectionConfig, Engine, RemoteCommand, RemoteService},
    remote_sql::SqlSession,
    transfer::TransferRequest,
};
use sqlx_core::{connection::Connection, query::query, row::Row};
use std::{
    io::{BufRead, BufReader},
    time::{Duration, Instant},
};
fn config(port: u16) -> ConnectionConfig {
    ConnectionConfig {
        id: "export-fixture".into(),
        name: "CSV test fixture".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: "root".into(),
        tls: false,
        read_only: true,
        timeout_secs: 3,
        idle_secs: 60,
        row_limit: 5,
        ..ConnectionConfig::default()
    }
}
fn request(path: std::path::PathBuf, sql: &str) -> QueryExportRequest {
    QueryExportRequest {
        path,
        sql: sql.into(),
        options: CsvExportOptions::default(),
        timeout_secs: 60,
    }
}
fn drain(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(70);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
#[ignore = "requires an explicitly started disposable MySQL fixture and CODISSEU_SQL_MY_PORT"]
fn mysql_streams_one_million_and_cancels_with_cleanup() {
    let port = std::env::var("CODISSEU_SQL_MY_PORT")
        .expect("disposable port")
        .parse::<u16>()
        .unwrap();
    let cfg = config(port);
    let dir = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(dir.path().join("profiles.json"));
    drain(&mut service);
    service
        .dispatch(RemoteCommand::Connect {
            config: cfg.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        })
        .unwrap();
    drain(&mut service);
    assert!(service.active.is_some(), "{}", service.status);
    service
        .dispatch(RemoteCommand::Execute("SELECT 77 AS preview".into()))
        .unwrap();
    drain(&mut service);
    assert!(service.error.is_none(), "{}", service.status);
    let revision = service.revision;
    let sql = "WITH digit AS (SELECT 0 AS n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) SELECT 1+a.n+10*b.n+100*c.n+1000*d.n+10000*e.n+100000*f.n AS n FROM digit a CROSS JOIN digit b CROSS JOIN digit c CROSS JOIN digit d CROSS JOIN digit e CROSS JOIN digit f";
    let path = dir.path().join("million.csv");
    service
        .dispatch(RemoteCommand::ExportQuery(TransferRequest {
            datasource_id: cfg.id.clone(),
            session_generation: service.session_generation,
            payload: request(path.clone(), sql),
        }))
        .unwrap();
    drain(&mut service);
    assert!(service.error.is_none(), "{}", service.status);
    let report = service.export_report.as_ref().unwrap();
    assert_eq!(report.rows, 1_000_000);
    assert_eq!(report.bytes, std::fs::metadata(&path).unwrap().len());
    assert_eq!(service.revision, revision);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        codisseu_db::remote::Cell::Number("77".into())
    );
    assert!(!service.transaction);
    let mut lines = BufReader::new(std::fs::File::open(&path).unwrap()).lines();
    assert_eq!(lines.next().unwrap().unwrap(), "n");
    let mut seen = vec![false; 1_000_000];
    for line in lines {
        let n = line.unwrap().parse::<usize>().unwrap();
        assert!((1..=1_000_000).contains(&n));
        assert!(!seen[n - 1]);
        seen[n - 1] = true;
    }
    assert!(seen.into_iter().all(|v| v));
    eprintln!(
        "MySQL CSV: {} rows, {} bytes, backend {} ms",
        report.rows, report.bytes, report.elapsed_ms
    );
    std::fs::remove_file(path).unwrap();
    let cancel_path = dir.path().join("cancel.csv");
    let big = sql
        .replace(" AS n FROM digit", "+1000000*g.n AS n FROM digit")
        .replace(
            "CROSS JOIN digit f",
            "CROSS JOIN digit f CROSS JOIN digit g",
        );
    service
        .dispatch(RemoteCommand::ExportQuery(TransferRequest {
            datasource_id: cfg.id.clone(),
            session_generation: service.session_generation,
            payload: request(cancel_path.clone(), &big),
        }))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.export_progress.snapshot().rows < 1024 {
        service.poll();
        assert!(service.is_busy(), "{}", service.status);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    service.dispatch(RemoteCommand::Cancel).unwrap();
    drain(&mut service);
    assert!(service.export_report.is_none());
    assert!(!cancel_path.exists());
    assert!(service.active.is_none());
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".codisseu-export-")
    }));
}
#[test]
#[ignore = "requires an explicitly started disposable MySQL fixture and CODISSEU_SQL_MY_PORT"]
fn mysql_export_enforces_server_readonly_even_for_write_config_and_functions() {
    let port = std::env::var("CODISSEU_SQL_MY_PORT")
        .expect("disposable port")
        .parse::<u16>()
        .unwrap();
    let mut cfg = config(port);
    cfg.read_only = false;
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let opts=sqlx_mysql::MySqlConnectOptions::new().host("127.0.0.1").port(port).username("root").password("codisseu_fixture").database("codisseu");let mut admin=sqlx_mysql::MySqlConnection::connect_with(&opts).await.unwrap();
        sqlx_core::raw_sql::raw_sql("DROP FUNCTION IF EXISTS codisseu_export_mutate").execute(&mut admin).await.unwrap();query("DROP TABLE IF EXISTS codisseu_export_guard").execute(&mut admin).await.unwrap();
        query("CREATE TABLE codisseu_export_guard(n BIGINT) ENGINE=InnoDB").execute(&mut admin).await.unwrap();
        sqlx_core::raw_sql::raw_sql("CREATE FUNCTION codisseu_export_mutate() RETURNS BIGINT DETERMINISTIC MODIFIES SQL DATA BEGIN INSERT INTO codisseu_export_guard VALUES(1); RETURN 1; END").execute(&mut admin).await.unwrap();
        let mut session=SqlSession::connect(&cfg,"codisseu_fixture").await.unwrap();let path=dir.path().join("guard.csv");
        let explain_mutation=session.explain("SELECT codisseu_export_mutate()").await;
        assert!(!session.transaction_active());
        let outcome=session.export_query(&request(path.clone(),"SELECT codisseu_export_mutate()"),QueryExportProgress::default()).await;
        let count:i64=query("SELECT count(*) AS n FROM codisseu_export_guard").fetch_one(&mut admin).await.unwrap().try_get("n").unwrap();
        sqlx_core::raw_sql::raw_sql("DROP FUNCTION codisseu_export_mutate").execute(&mut admin).await.unwrap();query("DROP TABLE codisseu_export_guard").execute(&mut admin).await.unwrap();admin.close().await.unwrap();
        assert!(explain_mutation.as_ref().is_err_and(|e|e.contains("25006")),"EXPLAIN function evaluation must be rejected by server read-only: {explain_mutation:?}");assert!(outcome.is_err(),"mutation accepted");assert_eq!(count,0);assert!(!path.exists());assert!(session.is_closed());
        let mut session=SqlSession::connect(&cfg,"codisseu_fixture").await.unwrap();
        let plan=session.explain("SELECT 1 AS n").await.unwrap();assert!(!plan.rows.is_empty());assert!(plan.columns.iter().any(|name|name=="select_type"),"MySQL plan column metadata must be visible: {:?}",plan.columns);assert_eq!(plan.columns.len(),plan.rows[0].len());assert!(!session.transaction_active());
        for sql in ["DELETE FROM users", "SELECT 1 INTO OUTFILE '/tmp/codisseu-forbidden'", "SELECT 1; SELECT 2", "WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x"] {assert!(session.export_query(&request(path.clone(),sql),QueryExportProgress::default()).await.is_err(),"{sql}");assert!(!path.exists());assert!(session.explain(sql).await.is_err(),"EXPLAIN {sql}");}
        session.execute("SELECT 1").await.unwrap();assert!(session.transaction_active());assert!(session.export_query(&request(path.clone(),"SELECT 1"),QueryExportProgress::default()).await.is_err());assert!(session.transaction_active());assert!(session.explain("SELECT 1").await.is_err());assert!(session.transaction_active());session.rollback().await.unwrap();
        session.export_query(&request(path.clone(),"SELECT -12 AS n LIMIT 1"),QueryExportProgress::default()).await.unwrap();assert!(!session.transaction_active());assert_eq!(std::fs::read_to_string(&path).unwrap(),"n\r\n-12\r\n");session.close().await.unwrap();
    });
}
