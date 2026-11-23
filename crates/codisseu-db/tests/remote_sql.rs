//! Disposable fixture tests. Run explicitly with CODISSEU_SQL_PG_PORT and
//! CODISSEU_SQL_MY_PORT pointing to loopback containers, never production.
use codisseu_db::remote::{Cell, CellEdit, ConnectionConfig, EditBatch, Engine, TableRef};
use codisseu_db::remote_sql::SqlSession;
use sqlx_core::{connection::Connection, query::query};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

fn config(engine: Engine, port: u16, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        id: "fixture".into(),
        name: "Disposable SQL fixture".into(),
        engine,
        host: "127.0.0.1".into(),
        port,
        database: "codisseu".into(),
        username: if matches!(engine, Engine::Postgres | Engine::AuroraPostgres) {
            "postgres".into()
        } else {
            "root".into()
        },
        tls: false,
        read_only,
        timeout_secs: 2,
        idle_secs: 60,
        row_limit: 5,
        ..ConnectionConfig::default()
    }
}
const HOSTILE_TABLES: [&str; 3] = [
    r"codisseu\quote'target",
    "codisseuquote'target",
    r"codisseu\\quote'target",
];
async fn seed(config: &ConnectionConfig) {
    let statements = [
        "DROP VIEW IF EXISTS codisseu_select_fixture_view",
        "DROP TABLE IF EXISTS codisseu_select_no_pk",
        "DROP TABLE IF EXISTS codisseu_fixture",
        "CREATE TABLE codisseu_fixture (id BIGINT PRIMARY KEY, name VARCHAR(100), amount DECIMAL(30,10), enabled BOOLEAN)",
        "INSERT INTO codisseu_fixture VALUES (1,'original',12345678901234567890.1234567890,true),(2,NULL,0,false)",
        "CREATE VIEW codisseu_select_fixture_view AS SELECT * FROM codisseu_fixture",
        "CREATE TABLE codisseu_select_no_pk (id BIGINT,name VARCHAR(100))",
        "INSERT INTO codisseu_select_no_pk VALUES(1,'ambiguous'),(1,'ambiguous')",
    ];
    if config.engine == Engine::Postgres {
        let options = sqlx_postgres::PgConnectOptions::new()
            .host("127.0.0.1")
            .port(config.port)
            .username("postgres")
            .password("codisseu_fixture")
            .database("codisseu");
        let mut connection = sqlx_postgres::PgConnection::connect_with(&options)
            .await
            .unwrap();
        for sql in statements {
            query(sql).execute(&mut connection).await.unwrap();
        }
        query("CREATE OR REPLACE FUNCTION codisseu_mutate() RETURNS BIGINT LANGUAGE sql AS 'UPDATE codisseu_fixture SET name=''unsafe'' WHERE id=1 RETURNING id'").execute(&mut connection).await.unwrap();
        for name in HOSTILE_TABLES {
            for sql in [
                format!("DROP TABLE IF EXISTS \"{name}\""),
                format!("CREATE TABLE \"{name}\"(id BIGINT PRIMARY KEY,name VARCHAR(100))"),
                format!("INSERT INTO \"{name}\" VALUES(1,'same original')"),
            ] {
                query(&sql).execute(&mut connection).await.unwrap();
            }
        }
        query("ALTER DATABASE codisseu SET standard_conforming_strings = off")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    } else {
        let options = sqlx_mysql::MySqlConnectOptions::new()
            .host("127.0.0.1")
            .port(config.port)
            .username("root")
            .password("codisseu_fixture")
            .database("codisseu");
        let mut connection = sqlx_mysql::MySqlConnection::connect_with(&options)
            .await
            .unwrap();
        for sql in statements {
            query(sql).execute(&mut connection).await.unwrap();
        }
        sqlx_core::raw_sql::raw_sql("DROP FUNCTION IF EXISTS codisseu_mutate")
            .execute(&mut connection)
            .await
            .unwrap();
        sqlx_core::raw_sql::raw_sql("CREATE FUNCTION codisseu_mutate() RETURNS BIGINT DETERMINISTIC MODIFIES SQL DATA BEGIN UPDATE codisseu_fixture SET name='unsafe' WHERE id=1; RETURN 1; END").execute(&mut connection).await.unwrap();
        for name in HOSTILE_TABLES {
            for sql in [
                format!("DROP TABLE IF EXISTS `{name}`"),
                format!("CREATE TABLE `{name}`(id BIGINT PRIMARY KEY,name VARCHAR(100))"),
                format!("INSERT INTO `{name}` VALUES(1,'same original')"),
            ] {
                query(&sql).execute(&mut connection).await.unwrap();
            }
        }
        query("SET GLOBAL sql_mode = CONCAT(@@GLOBAL.sql_mode,',NO_BACKSLASH_ESCAPES')")
            .execute(&mut connection)
            .await
            .unwrap();
        connection.close().await.unwrap();
    }
}
async fn exercise(engine: Engine, port: u16) {
    let mut cfg = config(engine, port, true);
    seed(&cfg).await;
    let table = TableRef {
        namespace: if engine == Engine::Postgres {
            "public".into()
        } else {
            "codisseu".into()
        },
        name: "codisseu_fixture".into(),
    };
    let mut read = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    if engine == Engine::Postgres {
        assert_eq!(
            read.execute("SELECT current_setting('standard_conforming_strings')")
                .await
                .unwrap()
                .rows[0][0],
            Cell::Text("off".into())
        );
    } else {
        let mode = read
            .execute("SELECT LOCATE('NO_BACKSLASH_ESCAPES',@@session.sql_mode)>0")
            .await
            .unwrap();
        assert!(
            matches!(&mode.rows[0][0], Cell::Bool(true))
                || mode.rows[0][0] == Cell::Number("1".into())
        );
    }
    let objects = read.inspect().await.unwrap();
    assert!(objects.iter().any(|o| o.name == table.name));
    assert!(
        objects.iter().all(|o| o.columns.is_empty()),
        "introspection is lazy"
    );
    let (result, columns) = read.load_table(&table).await.unwrap();
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Cell::Number("1".into()));
    assert_eq!(result.rows[1][1], Cell::Null);
    assert_eq!(
        result.rows[0][2],
        Cell::Number("12345678901234567890.1234567890".into())
    );
    assert!(columns[0].primary_key);
    let (second, _) = read.load_table_page(&table, 1).await.unwrap();
    assert_eq!(second.rows[0][0], Cell::Number("2".into()));
    assert!(read.load_table_page(&table, 1_000_001).await.is_err());
    assert!(!read.transaction_active());
    if engine == Engine::Postgres {
        let denial = read.execute("SELECT codisseu_mutate()").await.unwrap_err();
        assert!(
            denial.contains("25006"),
            "server must enforce read-only: {denial}"
        );
    } else {
        let denial = read.execute("SELECT codisseu_mutate()").await.unwrap_err();
        assert!(
            denial.contains("25006") || denial.contains("25000"),
            "MySQL server must enforce read-only: {denial}"
        );
    }
    for sql in [
        "DELETE FROM codisseu_fixture",
        "SELECT 1; COMMIT",
        "SELECT 1 FOR UPDATE",
        "SELECT set_config('transaction_read_only','off',true)",
    ] {
        assert!(read.execute(sql).await.is_err(), "{sql}");
    }
    let bounded=read.execute("SELECT a.id FROM codisseu_fixture a CROSS JOIN codisseu_fixture b CROSS JOIN codisseu_fixture c").await.unwrap();
    assert_eq!(bounded.rows.len(), 5);
    assert!(bounded.truncated);
    let json_query = if engine == Engine::Postgres {
        "SELECT CAST('{\"value\":123456789012345678901234567890.123456789}' AS JSONB)"
    } else {
        "SELECT CAST('{\"value\":18446744073709551615}' AS JSON)"
    };
    let json = read.execute(json_query).await.unwrap();
    let exact = if engine == Engine::Postgres {
        "123456789012345678901234567890.123456789"
    } else {
        "18446744073709551615"
    };
    assert!(matches!(&json.rows[0][0],Cell::Json(value) if value.contains(exact)));
    let oversized = read.execute("SELECT REPEAT('x',70000)").await.unwrap();
    assert!(oversized.rows.is_empty());
    assert!(oversized.truncated);
    let update = EditBatch {
        table: table.clone(),
        columns: columns.clone(),
        edits: vec![CellEdit {
            row: result.rows[0].clone(),
            column: "name".into(),
            value: Cell::Text("changed ' 💙".into()),
        }],
    };
    assert!(read.apply(&update).await.is_err());
    cfg.read_only = false;
    let mut writer = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    // All lookalikes deliberately have the same PK, schema and original values:
    // choosing the wrong table would otherwise produce a convincing successful UPDATE.
    let quoted_hostile = if engine == Engine::Postgres {
        format!("\"{}\"", HOSTILE_TABLES[0])
    } else {
        format!("`{}`", HOSTILE_TABLES[0])
    };
    let hostile_sql = format!("SELECT * FROM {quoted_hostile} WHERE id=1");
    let hostile_result = writer.execute(&hostile_sql).await.unwrap();
    let (hostile_table, hostile_columns) = writer
        .editable_origin(&hostile_sql, &hostile_result)
        .await
        .unwrap()
        .expect("hostile identifier must resolve exactly");
    assert_eq!(hostile_table.name, HOSTILE_TABLES[0]);
    let hostile_batch = EditBatch {
        table: hostile_table,
        columns: hostile_columns,
        edits: vec![CellEdit {
            row: hostile_result.rows[0].clone(),
            column: "name".into(),
            value: Cell::Text("only actual changed".into()),
        }],
    };
    assert_eq!(writer.apply(&hostile_batch).await.unwrap().affected_rows, 1);
    writer.commit().await.unwrap();
    for (index, name) in HOSTILE_TABLES.iter().enumerate() {
        let quoted = if engine == Engine::Postgres {
            format!("\"{name}\"")
        } else {
            format!("`{name}`")
        };
        assert_eq!(
            read.execute(&format!("SELECT name FROM {quoted} WHERE id=1"))
                .await
                .unwrap()
                .rows[0][0],
            Cell::Text(
                if index == 0 {
                    "only actual changed"
                } else {
                    "same original"
                }
                .into()
            )
        );
    }
    let own_select = format!(
        "SELECT * FROM {}.codisseu_fixture WHERE id=1 ORDER BY id LIMIT 1",
        table.namespace
    );
    let own_result = writer.execute(&own_select).await.unwrap();
    let (own_table, own_columns) = writer
        .editable_origin(&own_select, &own_result)
        .await
        .unwrap()
        .expect("plain own SELECT must certify a real PK row");
    assert_eq!(own_table, table);
    let own_batch = EditBatch {
        table: own_table,
        columns: own_columns,
        edits: vec![CellEdit {
            row: own_result.rows[0].clone(),
            column: "name".into(),
            value: Cell::Text("edited from own SELECT".into()),
        }],
    };
    assert_eq!(writer.apply(&own_batch).await.unwrap().affected_rows, 1);
    assert_eq!(
        writer
            .execute("SELECT name FROM codisseu_fixture WHERE id=1")
            .await
            .unwrap()
            .rows[0][0],
        Cell::Text("edited from own SELECT".into())
    );
    assert_eq!(
        read.execute("SELECT name FROM codisseu_fixture WHERE id=1")
            .await
            .unwrap()
            .rows[0][0],
        Cell::Text("original".into())
    );
    writer.rollback().await.unwrap();
    for sql in [
        "SELECT * FROM codisseu_fixture WHERE id=1",
        "SELECT t.* FROM codisseu_fixture AS t WHERE t.id=1",
        "SELECT t.id AS id,t.name,t.amount,t.enabled FROM codisseu_fixture t ORDER BY id LIMIT 2",
        "SELECT name,id,amount,enabled FROM codisseu_fixture",
        "SELECT name,id FROM codisseu_fixture",
    ] {
        let result = writer.execute(sql).await.unwrap();
        assert!(
            writer
                .editable_origin(sql, &result)
                .await
                .unwrap()
                .is_some(),
            "supported SELECT: {sql}"
        );
    }
    for sql in [
        "SELECT a.* FROM codisseu_fixture a JOIN codisseu_fixture b ON a.id=b.id",
        "SELECT id,name,amount+0 AS amount,enabled FROM codisseu_fixture",
        "SELECT COUNT(*) FROM codisseu_fixture",
        "SELECT name,amount,enabled FROM codisseu_fixture",
        "SELECT name AS id,id AS name,amount,enabled FROM codisseu_fixture",
        "SELECT DISTINCT * FROM codisseu_fixture",
        "SELECT id,name,amount,enabled FROM codisseu_fixture GROUP BY id,name,amount,enabled",
        "WITH t AS(SELECT * FROM codisseu_fixture) SELECT * FROM t",
        "SELECT * FROM (SELECT * FROM codisseu_fixture) t",
        "SELECT * FROM codisseu_fixture UNION ALL SELECT * FROM codisseu_fixture",
        "SELECT * FROM codisseu_select_fixture_view",
        "SELECT * FROM codisseu_select_no_pk",
    ] {
        let result = writer.execute(sql).await.unwrap();
        assert!(
            writer
                .editable_origin(sql, &result)
                .await
                .unwrap()
                .is_none(),
            "ambiguous SELECT must be readonly: {sql}"
        );
    }
    writer.rollback().await.unwrap();
    let applied = writer.apply(&update).await.unwrap();
    assert_eq!(applied.affected_rows, 1);
    assert!(writer.transaction_active());
    let query = format!(
        "SELECT name FROM {}.codisseu_fixture WHERE id=1",
        table.namespace
    );
    assert_eq!(
        writer.execute(&query).await.unwrap().rows[0][0],
        Cell::Text("changed ' 💙".into())
    );
    assert_eq!(
        read.execute(&query).await.unwrap().rows[0][0],
        Cell::Text("original".into())
    );
    writer.rollback().await.unwrap();
    assert_eq!(
        read.execute(&query).await.unwrap().rows[0][0],
        Cell::Text("original".into())
    );
    writer.apply(&update).await.unwrap();
    writer.commit().await.unwrap();
    assert_eq!(
        read.execute(&query).await.unwrap().rows[0][0],
        Cell::Text("changed ' 💙".into())
    );
    // Stale original does not overwrite a concurrently committed value.
    assert!(
        writer
            .apply(&update)
            .await
            .unwrap_err()
            .contains("Conflito")
    );
    writer.rollback().await.unwrap();
    assert_eq!(
        read.execute(&query).await.unwrap().rows[0][0],
        Cell::Text("changed ' 💙".into())
    );
    let (current, columns) = writer.load_table(&table).await.unwrap();
    let revert = EditBatch {
        table: table.clone(),
        columns,
        edits: vec![CellEdit {
            row: current.rows[0].clone(),
            column: "name".into(),
            value: Cell::Text("uncommitted".into()),
        }],
    };
    writer.apply(&revert).await.unwrap();
    let mut contended = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    let lock_started = std::time::Instant::now();
    let lock_error = contended.apply(&revert).await.unwrap_err();
    assert!(
        lock_error.contains("55P03") || lock_error.contains("1205"),
        "expected server lock deadline: {lock_error}"
    );
    assert!(lock_started.elapsed() < Duration::from_secs(4));
    contended.close().await.unwrap();
    writer.close().await.unwrap();
    assert_eq!(
        read.execute(&query).await.unwrap().rows[0][0],
        Cell::Text("changed ' 💙".into())
    );
    read.close().await.unwrap();
    assert!(read.is_closed());
    // Cancellation closes the only session; do not label server cancellation as confirmed.
    let mut cancel_session = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    let cancel = Arc::new(AtomicBool::new(true));
    cancel_session.set_cancel_flag(cancel);
    assert!(cancel_session.execute("SELECT 1").await.is_err());
    assert!(cancel_session.is_closed());
    // Deadline is enforced for an expensive read at the server and/or transport.
    cfg.timeout_secs = 1;
    let mut timed = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    let sql = if engine == Engine::Postgres {
        "SELECT SUM(CAST(a.n AS BIGINT)*b.n) FROM generate_series(1,100000) a(n) CROSS JOIN generate_series(1,100000) b(n)"
    } else {
        "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<999) SELECT SUM(a.x*b.x*c.x) FROM n a CROSS JOIN n b CROSS JOIN n c"
    };
    let start = std::time::Instant::now();
    let timeout_error = timed.execute(sql).await.unwrap_err();
    assert!(
        timeout_error.contains("57014")
            || timeout_error.contains("3024")
            || timeout_error.contains("Prazo"),
        "expected actual deadline: {timeout_error}"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    timed.close().await.ok();
    // No-op updates produce no statement, avoiding affected-row ambiguity.
    let mut no_op = update.clone();
    no_op.edits[0].value = no_op.edits[0].row[1].clone();
    let mut session = SqlSession::connect(&cfg, "codisseu_fixture").await.unwrap();
    assert_eq!(session.apply(&no_op).await.unwrap().affected_rows, 0);
    session.close().await.unwrap();
}
#[test]
#[ignore = "requires disposable PostgreSQL 17 and MySQL 8.4 containers on explicit loopback ports"]
fn native_remote_sql_fixture() {
    let pg = std::env::var("CODISSEU_SQL_PG_PORT")
        .expect("explicit disposable PG port")
        .parse()
        .unwrap();
    let my = std::env::var("CODISSEU_SQL_MY_PORT")
        .expect("explicit disposable MySQL port")
        .parse()
        .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        exercise(Engine::Postgres, pg).await;
        exercise(Engine::Mysql, my).await;
    });
}
