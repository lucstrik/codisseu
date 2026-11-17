//! Opt-in disposable loopback MySQL only. The credential is a public fixture
//! password; no saved profiles, OS credential store or production source is used.
use codisseu_db::{
    query_export::{CsvExportOptions, QueryExportRequest},
    query_page::{ColumnSort, MAX_PAGE_OFFSET, QueryPageRequest},
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand, RemoteService},
    remote_sql::SqlSession,
    transfer::TransferRequest,
};
use sqlx_core::{connection::Connection, query::query};
use std::time::{Duration, Instant};

fn config() -> ConnectionConfig {
    let port = std::env::var("CODISSEU_SQL_MY_PORT")
        .expect("explicit disposable MySQL loopback port")
        .parse::<u16>()
        .unwrap();
    ConnectionConfig {
        id: "navigator-fixture".into(),
        name: "Disposable query navigation".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: "root".into(),
        tls: false,
        read_only: true,
        timeout_secs: 3,
        idle_secs: 60,
        row_limit: 500,
        save_credentials: false,
        ..ConnectionConfig::default()
    }
}

async fn admin(config: &ConnectionConfig) -> sqlx_mysql::MySqlConnection {
    let options = sqlx_mysql::MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(config.port)
        .database("codisseu")
        .username("root")
        .password("codisseu_fixture");
    sqlx_mysql::MySqlConnection::connect_with(&options)
        .await
        .unwrap()
}

async fn seed(admin: &mut sqlx_mysql::MySqlConnection, table: &str) {
    query(&format!("DROP TABLE IF EXISTS {table}"))
        .execute(&mut *admin)
        .await
        .unwrap();
    query(&format!(
        "CREATE TABLE {table}(id BIGINT PRIMARY KEY,label VARCHAR(32)) ENGINE=InnoDB"
    ))
    .execute(&mut *admin)
    .await
    .unwrap();
    query(&format!("INSERT INTO {table}(id,label) WITH digit AS (SELECT 0 n UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9), numbers AS (SELECT 1+a.n+10*b.n+100*c.n+1000*d.n AS n FROM digit a CROSS JOIN digit b CROSS JOIN digit c CROSS JOIN digit d WHERE 1+a.n+10*b.n+100*c.n+1000*d.n<=1205) SELECT n, CONCAT('sample-',LPAD(n,4,'0')) FROM numbers"))
        .execute(&mut *admin)
        .await
        .unwrap();
}

fn request(sql: &str, offset: u64, sort: Option<ColumnSort>) -> QueryPageRequest {
    QueryPageRequest {
        sql: sql.into(),
        offset,
        sort,
    }
}

#[test]
#[ignore = "requires disposable MySQL on explicit CODISSEU_SQL_MY_PORT"]
fn mysql_query_pages_sort_the_complete_result_and_preserve_original_scope() {
    let config = config();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let mut writer = admin(&config).await;
        seed(&mut writer, "navigator_paging").await;
        let mut session = SqlSession::connect(&config, "codisseu_fixture").await.unwrap();
        let sql = "SELECT id,label FROM navigator_paging ORDER BY id";
        let mut all = Vec::new();
        for (offset, expected, more) in [(0,500,true),(500,500,true),(1000,205,false)] {
            let page = session.execute_page(&request(sql, offset, None)).await.unwrap();
            assert_eq!(page.rows.len(), expected);
            assert_eq!(page.truncated, more);
            assert!(!session.transaction_active(), "read-only pages must not retain server snapshots");
            all.extend(page.rows.into_iter().map(|row| row[0].clone()));
        }
        assert_eq!(all, (1..=1205).map(|i| Cell::Number(i.to_string())).collect::<Vec<_>>());
        let descending = Some(ColumnSort { column: 0, descending: true });
        let page = session.execute_page(&request(sql, 0, descending)).await.unwrap();
        assert_eq!(page.rows[0][0], Cell::Number("1205".into()));
        assert_eq!(page.rows[499][0], Cell::Number("706".into()));
        assert_eq!(session.execute_page(&request(sql,500,descending)).await.unwrap().rows[0][0], Cell::Number("705".into()));
        let page = session.execute_page(&request(sql,1000,Some(ColumnSort { column: 1, descending: false }))).await.unwrap();
        assert_eq!(page.rows[0][1], Cell::Text("sample-1001".into()));
        let bounded = "WITH subset AS (SELECT id FROM navigator_paging ORDER BY id LIMIT 600 OFFSET 100) SELECT id,'a;b' AS text FROM subset; -- trailing ;";
        let page = session.execute_page(&request(bounded,500,None)).await.unwrap();
        assert_eq!(page.rows.len(),100);
        assert_eq!(page.rows[0],vec![Cell::Number("601".into()),Cell::Text("a;b".into())]);
        assert!(!page.truncated);
        let exact = session.execute_page(&request("SELECT id FROM navigator_paging ORDER BY id LIMIT 500",0,None)).await.unwrap();
        assert_eq!(exact.rows.len(),500);
        assert!(!exact.truncated);
        assert!(session.execute_page(&request(sql,1500,None)).await.unwrap().rows.is_empty());
        let oversized = session.execute_page(&request("SELECT id,REPEAT('x',60000) AS payload FROM navigator_paging WHERE id<=100 ORDER BY id",0,None)).await.unwrap();
        assert!(oversized.truncated && !oversized.rows.is_empty() && oversized.rows.len()<500);
        let next = session.execute_page(&request("SELECT id,REPEAT('x',60000) AS payload FROM navigator_paging WHERE id<=100 ORDER BY id",oversized.rows.len() as u64,None)).await.unwrap();
        assert_eq!(next.rows[0][0],Cell::Number((oversized.rows.len()+1).to_string()));
        assert_eq!(oversized.rows.len()+next.rows.len(),100);
        for invalid in [request(sql,MAX_PAGE_OFFSET+1,None),request(sql,0,Some(ColumnSort{column:256,descending:false})), request("SELECT id FROM navigator_paging",0,Some(ColumnSort{column:1,descending:false}))] {
            assert!(session.execute_page(&invalid).await.is_err());
        }
        for invalid in ["DELETE FROM navigator_paging", "SELECT 1; SELECT 2", "SELECT id FROM navigator_paging FOR UPDATE", "SELECT 1 INTO OUTFILE '/tmp/forbidden-navigator'", "SELECT unknown FROM navigator_paging"] {
            assert!(session.execute_page(&request(invalid,0,None)).await.is_err(),"accepted {invalid}");
        }
        assert_eq!(session.execute_page(&request("SELECT COUNT(*) FROM navigator_paging",0,None)).await.unwrap().rows[0][0],Cell::Number("1205".into()));
        session.close().await.unwrap();
        query("DROP TABLE navigator_paging").execute(&mut writer).await.unwrap();
        writer.close().await.unwrap();
    });
}

fn drain(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}

fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    drain(service);
    assert!(service.error.is_none(), "{}", service.status);
}

#[test]
#[ignore = "requires disposable MySQL on explicit CODISSEU_SQL_MY_PORT"]
fn mysql_worker_binds_pages_to_source_and_generation_and_exports_the_original_query() {
    let mut config = config();
    config.idle_secs = 5;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut writer = rt.block_on(admin(&config));
    rt.block_on(seed(&mut writer, "navigator_service"));
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("profiles.json"));
    drain(&mut service);
    send(
        &mut service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
    let sql = "SELECT id FROM navigator_service ORDER BY id";
    let page = codisseu_db::query_page::BoundQueryPage {
        datasource_id: config.id.clone(),
        connection_generation: service.connection_generation,
        payload: request(sql, 500, None),
    };
    send(&mut service, RemoteCommand::ExecutePage(page.clone()));
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Number("501".into())
    );
    let physical = service.session_generation;
    // Socket can expire before the GUI polls. Logical page identity remains valid.
    std::thread::sleep(Duration::from_millis(5400));
    send(&mut service, RemoteCommand::ExecutePage(page.clone()));
    assert_eq!(service.session_generation, physical + 1);
    assert_eq!(service.connection_generation, page.connection_generation);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Number("501".into())
    );
    let revision = service.revision;
    for invalid in [
        codisseu_db::query_page::BoundQueryPage {
            datasource_id: "another-source".into(),
            ..page.clone()
        },
        codisseu_db::query_page::BoundQueryPage {
            connection_generation: page.connection_generation + 1,
            ..page.clone()
        },
        codisseu_db::query_page::BoundQueryPage {
            payload: request(sql, MAX_PAGE_OFFSET + 1, None),
            ..page.clone()
        },
    ] {
        assert!(
            service
                .dispatch(RemoteCommand::ExecutePage(invalid))
                .is_err()
        );
        assert_eq!(service.revision, revision);
        assert!(!service.is_busy());
        assert_eq!(
            service.result.as_ref().unwrap().rows[0][0],
            Cell::Number("501".into())
        );
    }
    let export = directory.path().join("original.csv");
    let session_generation = service.session_generation;
    send(
        &mut service,
        RemoteCommand::ExportQuery(TransferRequest {
            datasource_id: config.id.clone(),
            session_generation,
            payload: QueryExportRequest {
                sql: sql.into(),
                path: export.clone(),
                options: CsvExportOptions::default(),
                timeout_secs: 10,
            },
        }),
    );
    assert_eq!(service.export_report.as_ref().unwrap().rows, 1205);
    let csv = std::fs::read_to_string(&export).unwrap();
    assert_eq!(csv.lines().count(), 1206);
    assert_eq!(csv.lines().nth(1), Some("1"));
    assert_eq!(csv.lines().last(), Some("1205"));
    assert_eq!(service.revision, revision);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Number("501".into())
    );
    assert!(!service.transaction);
    send(
        &mut service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
    assert_ne!(service.connection_generation, page.connection_generation);
    assert!(
        service.dispatch(RemoteCommand::ExecutePage(page)).is_err(),
        "old query page must not target a replacement session"
    );
    send(&mut service, RemoteCommand::Disconnect);
    drop(service);
    rt.block_on(query("DROP TABLE navigator_service").execute(&mut writer))
        .unwrap();
    rt.block_on(writer.close()).unwrap();
}
