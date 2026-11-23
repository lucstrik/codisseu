//! End-to-end worker tests against explicitly selected disposable loopback databases.
use codisseu_db::{
    cell_edit::parse_cell_edit,
    remote::{
        Cell, CellEdit, ConnectionConfig, EditBatch, Engine, RemoteCommand, RemoteService, TableRef,
    },
    remote_sql::SqlSession,
};
use sqlx_core::{connection::Connection, query::query};
use std::time::{Duration, Instant};

fn postgres(engine: Engine) -> bool {
    matches!(engine, Engine::Postgres | Engine::AuroraPostgres)
}

fn cfg(engine: Engine, port: u16) -> ConnectionConfig {
    ConnectionConfig {
        id: "grid-edit-fixture".into(),
        name: "Disposable grid editing".into(),
        engine,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: if postgres(engine) { "postgres" } else { "root" }.into(),
        tls: false,
        read_only: false,
        timeout_secs: 4,
        idle_secs: 60,
        row_limit: 50,
        ..Default::default()
    }
}
async fn write(config: &ConnectionConfig, sql: &str) {
    if postgres(config.engine) {
        let options = sqlx_postgres::PgConnectOptions::new()
            .host("127.0.0.1")
            .port(config.port)
            .database("codisseu")
            .username("postgres")
            .password("codisseu_fixture");
        let mut c = sqlx_postgres::PgConnection::connect_with(&options)
            .await
            .unwrap();
        query(sql).execute(&mut c).await.unwrap();
        c.close().await.unwrap();
    } else {
        let options = sqlx_mysql::MySqlConnectOptions::new()
            .host("127.0.0.1")
            .port(config.port)
            .database("codisseu")
            .username("root")
            .password("codisseu_fixture");
        let mut c = sqlx_mysql::MySqlConnection::connect_with(&options)
            .await
            .unwrap();
        query(sql).execute(&mut c).await.unwrap();
        c.close().await.unwrap();
    }
}
fn idle(service: &mut RemoteService) {
    let end = Instant::now() + Duration::from_secs(12);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < end,
            "worker busy: {} {:?}",
            service.status,
            service.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
#[track_caller]
fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    idle(service);
    assert!(
        service.error.is_none(),
        "{} {:?}",
        service.status,
        service.error
    );
}
#[track_caller]
fn execute(service: &mut RemoteService, sql: &str) {
    send(service, RemoteCommand::Execute(sql.into()));
}
fn batch(service: &RemoteService, changes: &[(usize, &str, Cell)]) -> EditBatch {
    let (table, columns) = service.origin.clone().expect("certified editable SELECT");
    let rows = &service.result.as_ref().unwrap().rows;
    EditBatch {
        table,
        columns,
        edits: changes
            .iter()
            .map(|(row, column, value)| CellEdit {
                row: rows[*row].clone(),
                column: (*column).into(),
                value: value.clone(),
            })
            .collect(),
    }
}
fn fixture(engine: Engine, port: u16) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let config = cfg(engine, port);
    rt.block_on(async {
        write(&config,"DROP TABLE IF EXISTS codisseu_grid_typed").await;
        let datetime=if postgres(engine) {"timestamp(6)"} else {"datetime(6)"};
        let json=if postgres(engine) {"jsonb"} else {"json"};
        let uuid=if postgres(engine) {"uuid"} else {"varchar(36)"};
        let magnitude=if postgres(engine) {"bigint"} else {"bigint unsigned"};
        let status=if postgres(engine) {"varchar(12)"} else {"enum('new','done')"};
        write(&config,&format!("CREATE TABLE codisseu_grid_typed(tenant BIGINT NOT NULL,id BIGINT NOT NULL,name VARCHAR(12) NOT NULL,qty SMALLINT NULL CHECK(qty>=0),amount DECIMAL(30,10),flag BOOLEAN NULL,created {datetime},calculated INTEGER GENERATED ALWAYS AS (qty+1) STORED,payload {json},uid {uuid},clock_time TIME(3),magnitude {magnitude},status {status},event_date DATE,PRIMARY KEY(tenant,id))")).await;
        write(&config,"INSERT INTO codisseu_grid_typed(tenant,id,name,qty,amount,flag,created) VALUES(1,1,'original',NULL,12345678901234567890.1234567890,NULL,'2026-11-23 10:11:12.120000'),(1,2,'untouched',5,1.0000000000,false,'2026-11-23 10:11:12.000000'),(2,1,'other tenant',7,2.0000000000,true,NULL)").await;
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("datasources.json");
    std::fs::write(&path, serde_json::to_vec(&vec![config.clone()]).unwrap()).unwrap();
    let mut service = RemoteService::new(path);
    idle(&mut service);
    send(
        &mut service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
    let sql = "SELECT * FROM codisseu_grid_typed ORDER BY tenant,id";
    execute(&mut service, sql);
    assert!(!service.transaction);
    let (_, columns) = service.origin.as_ref().unwrap();
    assert_eq!(columns.iter().filter(|c| c.primary_key).count(), 2);
    assert_eq!(
        parse_cell_edit(
            columns.iter().find(|c| c.name == "qty").unwrap(),
            "42",
            false
        )
        .unwrap(),
        Cell::Number("42".into())
    );
    assert!(
        parse_cell_edit(
            columns.iter().find(|c| c.name == "qty").unwrap(),
            "text",
            false
        )
        .is_err()
    );
    assert!(parse_cell_edit(columns.iter().find(|c| c.name == "name").unwrap(), "", true).is_err());
    assert!(
        parse_cell_edit(
            columns.iter().find(|c| c.name == "name").unwrap(),
            "this is far too long",
            false
        )
        .is_err()
    );
    let generated = columns.iter().find(|c| c.name == "calculated").unwrap();
    assert!(generated.data_type.starts_with("generated "));
    assert!(parse_cell_edit(generated, "9", false).is_err());
    let mut observer_cfg = config.clone();
    observer_cfg.read_only = true;
    let mut observer = rt
        .block_on(SqlSession::connect(&observer_cfg, "codisseu_fixture"))
        .unwrap();
    let mariadb = !postgres(engine)
        && rt
            .block_on(observer.execute("SELECT VERSION()"))
            .unwrap()
            .rows[0][0]
            .clipboard_text()
            .contains("MariaDB");
    let read_name = |observer: &mut SqlSession| {
        rt.block_on(
            observer.execute("SELECT name FROM codisseu_grid_typed WHERE tenant=1 AND id=1"),
        )
        .unwrap()
        .rows[0][0]
            .clone()
    };
    let initial = batch(
        &service,
        &[
            (0, "name", Cell::Text("edited ' 💙".into())),
            (0, "qty", Cell::Number("42".into())),
        ],
    );
    send(&mut service, RemoteCommand::Apply(initial));
    assert!(service.transaction);
    assert_eq!(read_name(&mut observer), Cell::Text("original".into()));
    send(&mut service, RemoteCommand::Rollback);
    assert!(!service.transaction);
    assert_eq!(read_name(&mut observer), Cell::Text("original".into()));
    execute(&mut service, sql);
    let commit = batch(
        &service,
        &[
            (0, "name", Cell::Text("committed".into())),
            (0, "qty", Cell::Number("42".into())),
            (
                0,
                "amount",
                Cell::Number("12345678901234567890.1234567891".into()),
            ),
        ],
    );
    send(&mut service, RemoteCommand::ApplyAndCommit(commit));
    assert!(!service.transaction);
    assert!(service.status.contains("Commit"));
    assert_eq!(read_name(&mut observer), Cell::Text("committed".into()));
    let outside = rt.block_on(observer.execute(sql)).unwrap();
    assert_eq!(outside.rows[1][2], Cell::Text("untouched".into()));
    assert_eq!(outside.rows[2][2], Cell::Text("other tenant".into()));
    assert_eq!(
        outside.rows[0][4],
        Cell::Number("12345678901234567890.1234567891".into())
    );
    // Both changes to one row form a single UPDATE; compound PK targets only tenant1/id1.
    assert_eq!(outside.rows[0][3], Cell::Number("42".into()));
    assert_eq!(outside.rows[0][7], Cell::Number("43".into()));
    // Nullable originals must use declared types, including JSON, UUID, native
    // temporal precision and the maximum signed/unsigned integer value.
    execute(&mut service, sql);
    let maximum = if postgres(engine) {
        "9223372036854775807"
    } else {
        "18446744073709551615"
    };
    let typed = batch(&service, &[
        (0,"qty",Cell::Null),
        (0,"flag",Cell::Bool(true)),
        (0,"created",Cell::Text("2026-11-23 23:59:59.123456".into())),
        (0,"payload",Cell::Json("{\"integer\":18446744073709551615,\"decimal\":1234567890.12345678901234567890,\"exact_decimal\":\"1234567890.12345678901234567890\"}".into())),
        (0,"uid",Cell::Text("550e8400-e29b-41d4-a716-446655440000".into())),
        (0,"clock_time",Cell::Text("12:34:56.123".into())),
        (0,"magnitude",Cell::Number(maximum.into())),
        (0,"status",Cell::Text("done".into())),
        (0,"event_date",Cell::Text("2024-02-29".into())),
    ]);
    let typed_preview = codisseu_db::remote::preview(&typed, engine).unwrap();
    service
        .dispatch(RemoteCommand::ApplyAndCommit(typed))
        .unwrap();
    idle(&mut service);
    assert!(
        service.error.is_none(),
        "{:?}\n{}",
        service.error,
        typed_preview
    );
    let typed = rt
        .block_on(
            observer.load_table(&TableRef {
                namespace: if postgres(engine) {
                    "public"
                } else {
                    "codisseu"
                }
                .into(),
                name: "codisseu_grid_typed".into(),
            }),
        )
        .unwrap()
        .0;
    assert_eq!(typed.rows[0][3], Cell::Null);
    assert!(
        matches!(&typed.rows[0][5], Cell::Bool(true))
            || typed.rows[0][5] == Cell::Number("1".into())
    );
    assert_eq!(
        typed.rows[0][6],
        Cell::Text("2026-11-23 23:59:59.123456".into())
    );
    assert!(
        typed.rows[0][8]
            .clipboard_text()
            .contains("18446744073709551615")
    );
    let payload = typed.rows[0][8].clipboard_text();
    // MySQL native JSON parses fractional literals as DOUBLE. This is server
    // normalization, verified independently through its CLI; text decimals stay exact.
    let numeric = if postgres(engine) || mariadb {
        "1234567890.12345678901234567890"
    } else {
        "1234567890.123457"
    };
    let parsed: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_str(&payload).unwrap();
    assert_eq!(parsed["decimal"].get(), numeric);
    assert_eq!(
        serde_json::from_str::<String>(parsed["exact_decimal"].get()).unwrap(),
        "1234567890.12345678901234567890"
    );
    assert_eq!(
        typed.rows[0][9],
        Cell::Text("550e8400-e29b-41d4-a716-446655440000".into())
    );
    assert_eq!(
        sqlx_core::types::chrono::NaiveTime::parse_from_str(
            &typed.rows[0][10].clipboard_text(),
            "%H:%M:%S%.f"
        )
        .unwrap(),
        sqlx_core::types::chrono::NaiveTime::from_hms_milli_opt(12, 34, 56, 123).unwrap()
    );
    assert_eq!(typed.rows[0][11], Cell::Number(maximum.into()));
    assert_eq!(typed.rows[0][12], Cell::Text("done".into()));
    assert_eq!(typed.rows[0][13], Cell::Text("2024-02-29".into()));
    // Projected and reordered results retain complete PK and original column names.
    execute(
        &mut service,
        "SELECT name,id,tenant,qty FROM codisseu_grid_typed WHERE tenant=1 ORDER BY id",
    );
    let projected = batch(&service, &[(0, "name", Cell::Text("projected".into()))]);
    send(&mut service, RemoteCommand::ApplyAndCommit(projected));
    assert_eq!(read_name(&mut observer), Cell::Text("projected".into()));
    for ambiguous in [
        "SELECT name,id FROM codisseu_grid_typed",
        "SELECT name AS id,id AS name,tenant FROM codisseu_grid_typed",
        "SELECT a.* FROM codisseu_grid_typed a JOIN codisseu_grid_typed b ON a.id=b.id",
        "SELECT DISTINCT * FROM codisseu_grid_typed",
        "SELECT tenant,id,qty+1 AS qty FROM codisseu_grid_typed",
    ] {
        execute(&mut service, ambiguous);
        assert!(service.origin.is_none(), "{ambiguous}");
    }
    // Invalid typed direct commands are blocked by worker validation as well as UI.
    execute(&mut service, sql);
    let invalid = batch(&service, &[(0, "qty", Cell::Text("wrong type".into()))]);
    service
        .dispatch(RemoteCommand::ApplyAndCommit(invalid))
        .unwrap();
    idle(&mut service);
    assert!(service.error.is_some());
    send(&mut service, RemoteCommand::Rollback);
    execute(&mut service, sql);
    let generated = batch(&service, &[(0, "calculated", Cell::Number("900".into()))]);
    service
        .dispatch(RemoteCommand::ApplyAndCommit(generated))
        .unwrap();
    idle(&mut service);
    assert!(service.error.is_some());
    send(&mut service, RemoteCommand::Rollback);
    // A server CHECK failure in a later row reverts the complete batch.
    execute(&mut service, sql);
    let failed = batch(
        &service,
        &[
            (0, "name", Cell::Text("must revert".into())),
            (1, "qty", Cell::Number("-1".into())),
        ],
    );
    service
        .dispatch(RemoteCommand::ApplyAndCommit(failed))
        .unwrap();
    idle(&mut service);
    assert!(service.error.is_some());
    assert_eq!(read_name(&mut observer), Cell::Text("projected".into()));
    send(&mut service, RemoteCommand::Rollback);
    // Concurrent committed changes are detected instead of silently overwritten.
    execute(&mut service, sql);
    let stale = batch(&service, &[(0, "name", Cell::Text("stale edit".into()))]);
    rt.block_on(write(
        &config,
        "UPDATE codisseu_grid_typed SET name='concurrent' WHERE tenant=1 AND id=1",
    ));
    service
        .dispatch(RemoteCommand::ApplyAndCommit(stale))
        .unwrap();
    idle(&mut service);
    assert!(
        service
            .error
            .as_ref()
            .is_some_and(|e| e.contains("Conflito")),
        "{:?}",
        service.error
    );
    assert_eq!(read_name(&mut observer), Cell::Text("concurrent".into()));
    send(&mut service, RemoteCommand::Rollback);
    if postgres(engine) {
        rt.block_on(async {
            write(&config,"DROP TABLE IF EXISTS codisseu_grid_deferred").await;
            write(&config,"CREATE TABLE codisseu_grid_deferred(id BIGINT PRIMARY KEY,value BIGINT UNIQUE DEFERRABLE INITIALLY DEFERRED)").await;
            write(&config,"INSERT INTO codisseu_grid_deferred VALUES(1,1),(2,2)").await;
        });
        execute(
            &mut service,
            "SELECT * FROM codisseu_grid_deferred ORDER BY id",
        );
        let deferred = batch(&service, &[(0, "value", Cell::Number("2".into()))]);
        service
            .dispatch(RemoteCommand::ApplyAndCommit(deferred))
            .unwrap();
        idle(&mut service);
        assert!(
            service.error.as_ref().is_some_and(|e| e.contains("23505")),
            "{:?}",
            service.error
        );
        assert!(!service.status.starts_with("Commit concluído"));
        let outside = rt
            .block_on(observer.execute("SELECT * FROM codisseu_grid_deferred ORDER BY id"))
            .unwrap();
        assert_eq!(outside.rows[0][1], Cell::Number("1".into()));
        assert_eq!(outside.rows[1][1], Cell::Number("2".into()));
        send(
            &mut service,
            RemoteCommand::Connect {
                config: config.clone(),
                password: "codisseu_fixture".into(),
                test: false,
            },
        );
        rt.block_on(write(&config, "DROP TABLE codisseu_grid_deferred"));
    }
    if !postgres(engine) {
        // Text-bound BIGINT keys must be cast back to native integers. Comparing
        // a BIGINT with a bare text parameter can coerce both to DOUBLE and
        // confuse adjacent keys beyond 2^53.
        rt.block_on(async {
            write(&config,"DROP TABLE IF EXISTS codisseu_grid_largepk").await;
            write(&config,"CREATE TABLE codisseu_grid_largepk(id BIGINT UNSIGNED PRIMARY KEY,name VARCHAR(30),approx FLOAT)").await;
            write(&config,"INSERT INTO codisseu_grid_largepk VALUES(18446744073709551614,'same',12.3),(18446744073709551615,'same',12.3)").await;
        });
        execute(
            &mut service,
            "SELECT * FROM codisseu_grid_largepk ORDER BY id",
        );
        let precise = batch(&service, &[(1, "name", Cell::Text("only max key".into()))]);
        send(&mut service, RemoteCommand::ApplyAndCommit(precise));
        execute(
            &mut service,
            "SELECT * FROM codisseu_grid_largepk ORDER BY id",
        );
        assert_eq!(
            service.result.as_ref().unwrap().rows[0][1],
            Cell::Text("same".into())
        );
        assert_eq!(
            service.result.as_ref().unwrap().rows[1][1],
            Cell::Text("only max key".into())
        );
        rt.block_on(write(&config, "DROP TABLE codisseu_grid_largepk"));
        rt.block_on(async {
            write(&config,"DROP TABLE IF EXISTS codisseu_grid_bintext").await;
            write(&config,"CREATE TABLE codisseu_grid_bintext(id BIGINT PRIMARY KEY,name VARCHAR(30) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin,data BLOB)").await;
            write(&config,"INSERT INTO codisseu_grid_bintext VALUES(1,'á💙',X'00FF')").await;
        });
        execute(&mut service, "SELECT id,name FROM codisseu_grid_bintext");
        assert_eq!(
            service.result.as_ref().unwrap().rows[0][1],
            Cell::Text("á💙".into())
        );
        let text = batch(&service, &[(0, "name", Cell::Text("edited á💙".into()))]);
        send(&mut service, RemoteCommand::ApplyAndCommit(text));
        execute(&mut service, "SELECT id,name FROM codisseu_grid_bintext");
        assert_eq!(
            service.result.as_ref().unwrap().rows[0][1],
            Cell::Text("edited á💙".into())
        );
        execute(&mut service, "SELECT * FROM codisseu_grid_bintext");
        assert!(
            service.origin.is_none(),
            "actual BLOB must retain read-only protection"
        );
        assert_eq!(
            service.result.as_ref().unwrap().rows[0][2],
            Cell::Binary(vec![0, 255])
        );
        rt.block_on(write(&config, "DROP TABLE codisseu_grid_bintext"));
    }
    send(&mut service, RemoteCommand::Disconnect);
    rt.block_on(observer.close()).unwrap();
    rt.block_on(write(&config, "DROP TABLE codisseu_grid_typed"));
}
#[test]
#[ignore = "requires disposable PostgreSQL on CODISSEU_SQL_PG_PORT"]
fn grid_edit_postgres_fixture() {
    fixture(
        Engine::Postgres,
        std::env::var("CODISSEU_SQL_PG_PORT")
            .expect("fixture PG port")
            .parse()
            .unwrap(),
    );
}
#[test]
#[ignore = "requires disposable MySQL on CODISSEU_SQL_MY_PORT"]
fn grid_edit_mysql_fixture() {
    fixture(
        Engine::Mysql,
        std::env::var("CODISSEU_SQL_MY_PORT")
            .expect("fixture MySQL port")
            .parse()
            .unwrap(),
    );
}
#[test]
#[ignore = "requires disposable MariaDB on CODISSEU_SQL_MARIA_PORT"]
fn grid_edit_mariadb_fixture() {
    fixture(
        Engine::Mysql,
        std::env::var("CODISSEU_SQL_MARIA_PORT")
            .expect("fixture MariaDB port")
            .parse()
            .unwrap(),
    );
}

// These variants exercise registered adapter routing using local protocol peers.
// They explicitly do not establish actual AWS Aurora deployment compatibility.
#[test]
#[ignore = "local PostgreSQL protocol peer only, not actual AWS Aurora; CODISSEU_SQL_PG_PORT"]
fn grid_edit_aurora_postgres_local_protocol_fixture() {
    fixture(
        Engine::AuroraPostgres,
        std::env::var("CODISSEU_SQL_PG_PORT")
            .expect("fixture PG port")
            .parse()
            .unwrap(),
    );
}
#[test]
#[ignore = "local MySQL protocol peer only, not actual AWS Aurora; CODISSEU_SQL_MY_PORT"]
fn grid_edit_aurora_mysql_local_protocol_fixture() {
    fixture(
        Engine::AuroraMysql,
        std::env::var("CODISSEU_SQL_MY_PORT")
            .expect("fixture MySQL port")
            .parse()
            .unwrap(),
    );
}
