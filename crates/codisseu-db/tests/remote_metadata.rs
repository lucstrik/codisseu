//! Lazy catalog and recoverable-console scenarios against disposable loopback DBs.
use codisseu_db::remote::{
    CatalogRequest, Cell, CellEdit, ConnectionConfig, EditBatch, Engine, TableRef,
};
use codisseu_db::remote_sql::SqlSession;
use sqlx_core::{connection::Connection, query::query};

const PASSWORD: &str = "codisseu_fixture";
fn config(engine: Engine, port: u16, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        id: "metadata-fixture".into(),
        name: "Disposable metadata".into(),
        engine,
        host: "127.0.0.1".into(),
        port,
        username: if engine == Engine::Postgres {
            "postgres"
        } else {
            "root"
        }
        .into(),
        database: if engine == Engine::Postgres {
            "codisseu"
        } else {
            ""
        }
        .into(),
        tls: false,
        read_only,
        timeout_secs: 3,
        idle_secs: 60,
        row_limit: 1,
        ..ConnectionConfig::default()
    }
}
async fn seed(cfg: &ConnectionConfig) {
    let mut statements = Vec::new();
    for namespace in ["codisseu_one", "codisseu_two", "codisseu_many"] {
        if cfg.engine == Engine::Postgres {
            statements.push(format!("CREATE SCHEMA {namespace}"));
        } else {
            statements.push(format!("CREATE DATABASE {namespace}"));
        }
    }
    for (namespace, value) in [("codisseu_one", "first"), ("codisseu_two", "second")] {
        statements.extend([
            format!("CREATE TABLE {namespace}.items(id BIGINT PRIMARY KEY,name VARCHAR(100),extra VARCHAR(100))"),
            format!("INSERT INTO {namespace}.items VALUES (1,'{value}',NULL),(2,'other','value')"),
            format!("CREATE VIEW {namespace}.item_names AS SELECT name FROM {namespace}.items"),
        ]);
    }
    for index in 0..620 {
        statements.push(format!("CREATE TABLE codisseu_many.t{index:03}(id INT)"));
    }
    let quoted = if cfg.engine == Engine::Postgres {
        "\"odd%_quote'table\""
    } else {
        "`odd%_quote'table`"
    };
    statements.push(format!("CREATE TABLE codisseu_many.{quoted}(id INT)"));
    let columns = (0..257)
        .map(|index| format!("c{index} INT"))
        .collect::<Vec<_>>()
        .join(",");
    statements.push(format!("CREATE TABLE codisseu_two.wide({columns})"));
    if cfg.engine == Engine::Postgres {
        let mut connection = sqlx_postgres::PgConnection::connect_with(
            &sqlx_postgres::PgConnectOptions::new()
                .host("127.0.0.1")
                .port(cfg.port)
                .username("postgres")
                .password(PASSWORD)
                .database("codisseu"),
        )
        .await
        .unwrap();
        for statement in statements {
            query(&statement).execute(&mut connection).await.unwrap();
        }
        connection.close().await.unwrap();
    } else {
        let mut connection = sqlx_mysql::MySqlConnection::connect_with(
            &sqlx_mysql::MySqlConnectOptions::new()
                .host("127.0.0.1")
                .port(cfg.port)
                .username("root")
                .password(PASSWORD),
        )
        .await
        .unwrap();
        for statement in statements {
            query(&statement).execute(&mut connection).await.unwrap();
        }
        connection.close().await.unwrap();
    }
}
async fn exercise(engine: Engine, port: u16) {
    let mut cfg = config(engine, port, true);
    seed(&cfg).await;
    let mut read = SqlSession::connect(&cfg, PASSWORD).await.unwrap();
    let namespaces = read.namespaces().await.unwrap();
    assert!(namespaces.contains(&"codisseu_one".to_owned()));
    assert!(namespaces.contains(&"codisseu_two".to_owned()));
    let objects = read.inspect_namespace("codisseu_one").await.unwrap();
    assert_eq!(objects.len(), 2);
    assert!(
        objects
            .iter()
            .all(|object| object.namespace == "codisseu_one" && object.columns.is_empty())
    );
    assert!(objects.iter().any(|object| object.name == "items"));
    assert!(
        objects
            .iter()
            .any(|object| object.name == "item_names" && object.kind == "VIEW")
    );
    let table = TableRef {
        namespace: "codisseu_one".into(),
        name: "items".into(),
    };
    let columns = read.columns_for_table(&table).await.unwrap();
    assert_eq!(
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "extra"]
    );
    assert!(columns[0].primary_key);
    assert!(!columns[0].nullable);
    assert!(columns[2].nullable);
    for (namespace, expected) in [("codisseu_one", "first"), ("codisseu_two", "second")] {
        let result = read
            .execute(&format!("SELECT name FROM {namespace}.items WHERE id=1"))
            .await
            .unwrap();
        assert_eq!(result.rows[0][0], Cell::Text(expected.into()));
    }
    let first = read
        .inspect_namespace_page(CatalogRequest {
            namespace: "codisseu_many".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(first.objects.len(), 256);
    assert!(first.previous.is_none());
    assert!(first.next.is_some());
    let second = read
        .inspect_namespace_page(CatalogRequest {
            namespace: "codisseu_many".into(),
            after: first.next.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(second.objects.len(), 256);
    let third = read
        .inspect_namespace_page(CatalogRequest {
            namespace: "codisseu_many".into(),
            after: second.next.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(third.objects.len(), 109);
    assert!(third.next.is_none());
    let all: Vec<_> = first
        .objects
        .iter()
        .chain(&second.objects)
        .chain(&third.objects)
        .map(|o| o.name.clone())
        .collect();
    assert_eq!(all.len(), 621);
    assert_eq!(
        all.iter().collect::<std::collections::BTreeSet<_>>().len(),
        621
    );
    assert!(all.contains(&"t619".into()));
    let back = read
        .inspect_namespace_page(CatalogRequest {
            namespace: "codisseu_many".into(),
            before: third.previous.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        back.objects.iter().map(|o| &o.name).collect::<Vec<_>>(),
        second.objects.iter().map(|o| &o.name).collect::<Vec<_>>()
    );
    let back_first = read
        .inspect_namespace_page(CatalogRequest {
            namespace: "codisseu_many".into(),
            before: back.previous.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        back_first
            .objects
            .iter()
            .map(|o| &o.name)
            .collect::<Vec<_>>(),
        first.objects.iter().map(|o| &o.name).collect::<Vec<_>>()
    );
    assert!(back_first.previous.is_none());
    for (prefix, count) in [("t61", 10), ("t%", 0), ("t_", 0), ("odd%_quote'", 1)] {
        let page = read
            .inspect_namespace_page(CatalogRequest {
                namespace: "codisseu_many".into(),
                prefix: prefix.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.objects.len(), count, "literal prefix {prefix}");
        assert!(page.next.is_none());
    }
    assert!(
        read.inspect_namespace_page(CatalogRequest {
            namespace: "codisseu_many".into(),
            after: Some("t001".into()),
            before: Some("t003".into()),
            ..Default::default()
        })
        .await
        .is_err()
    );
    assert_eq!(
        read.inspect_namespace("codisseu_many").await.unwrap().len(),
        256
    );
    assert!(
        read.columns_for_table(&TableRef {
            namespace: "codisseu_two".into(),
            name: "wide".into()
        })
        .await
        .unwrap_err()
        .contains("256")
    );
    assert!(
        read.inspect_namespace("codisseu_one' OR '1'='1")
            .await
            .unwrap()
            .is_empty(),
        "namespace is a literal value"
    );
    assert!(read.inspect_namespace("").await.is_err());
    assert!(
        read.columns_for_table(&TableRef {
            namespace: "codisseu_two".into(),
            name: "missing".into()
        })
        .await
        .is_err()
    );
    assert!(!read.is_closed());
    let identity_query = if engine == Engine::Postgres {
        "SELECT pg_backend_pid()"
    } else {
        "SELECT CONNECTION_ID()"
    };
    let identity = read.execute(identity_query).await.unwrap().rows[0][0].clone();
    for sql in [
        "SELECT missing_column FROM codisseu_one.items",
        "SELECT * FROM codisseu_one.does_not_exist",
        "SELECT codisseu_one.items LIMIT 10",
    ] {
        let error = read.execute(sql).await.unwrap_err();
        assert!(error.contains("42"), "server error expected: {error}");
        assert!(!read.is_closed(), "statement typo must preserve connection");
        assert!(
            !read.transaction_active(),
            "read-only error rolls back snapshot"
        );
        assert_eq!(
            read.execute(identity_query).await.unwrap().rows[0][0],
            identity
        );
    }
    let limited = read
        .execute("SELECT * FROM codisseu_one.items ORDER BY id")
        .await
        .unwrap();
    assert_eq!(
        limited.rows.len(),
        1,
        "metadata limit must restore configured result limit"
    );
    assert!(limited.truncated);

    cfg.read_only = false;
    let mut writer = SqlSession::connect(&cfg, PASSWORD).await.unwrap();
    let (original, columns) = writer.load_table(&table).await.unwrap();
    let batch = EditBatch {
        table: table.clone(),
        columns,
        edits: vec![CellEdit {
            row: original.rows[0].clone(),
            column: "name".into(),
            value: Cell::Text("pending survives typo".into()),
        }],
    };
    writer.apply(&batch).await.unwrap();
    for sql in [
        "SELECT missing_column FROM codisseu_one.items",
        "SELECT * FROM codisseu_one.missing",
    ] {
        assert!(writer.execute(sql).await.is_err());
        assert!(!writer.is_closed());
        assert!(writer.transaction_active());
        assert_eq!(
            writer
                .execute("SELECT name FROM codisseu_one.items WHERE id=1")
                .await
                .unwrap()
                .rows[0][0],
            Cell::Text("pending survives typo".into())
        );
        assert_eq!(
            read.execute("SELECT name FROM codisseu_one.items WHERE id=1")
                .await
                .unwrap()
                .rows[0][0],
            Cell::Text("first".into()),
            "earlier pending write must remain uncommitted"
        );
    }
    // An internal metadata miss also recovers through the same savepoint path.
    assert!(
        writer
            .columns_for_table(&TableRef {
                namespace: "codisseu_one".into(),
                name: "missing".into()
            })
            .await
            .is_err()
    );
    writer.commit().await.unwrap();
    assert_eq!(
        read.execute("SELECT name FROM codisseu_one.items WHERE id=1")
            .await
            .unwrap()
            .rows[0][0],
        Cell::Text("pending survives typo".into())
    );
    writer.close().await.unwrap();
    read.close().await.unwrap();
}
#[test]
#[ignore = "requires explicit disposable PG/MySQL loopback ports; tools/db_metadata_fixtures.py"]
fn lazy_multi_namespace_catalog_and_statement_error_recovery() {
    let pg = std::env::var("CODISSEU_SQL_PG_PORT")
        .expect("disposable PG port")
        .parse()
        .unwrap();
    let my = std::env::var("CODISSEU_SQL_MY_PORT")
        .expect("disposable MySQL port")
        .parse()
        .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            exercise(Engine::Postgres, pg).await;
            exercise(Engine::Mysql, my).await;
        });
}
