//! Acceptance tests for Codisseu's own read-only SQLite contract.
use codisseu_db::{DbCommand, DbService, QUERY_LIMIT, Value, display_value, value_type_name};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

struct Lab {
    _directory: tempfile::TempDir,
    path: PathBuf,
}
impl Lab {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("readings.sqlite");
        let database = Connection::open(&path).unwrap();
        database.execute_batch("CREATE TABLE readings(sample INTEGER PRIMARY KEY, description TEXT, temperature REAL, encoded BLOB); INSERT INTO readings VALUES (10,'Medição α',-2.25,X'DEAD'),(20,'',NULL,NULL);").unwrap();
        Self {
            _directory: directory,
            path,
        }
    }
    fn service(&self) -> DbService {
        let mut service = DbService::new();
        send(&mut service, DbCommand::Connect(self.path.clone()));
        assert!(service.connected, "{:?}", service.last_error);
        assert!(service.last_error.is_none());
        service
    }
}

fn drain(service: &mut DbService) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        thread::sleep(Duration::from_millis(1));
    }
}
fn send(service: &mut DbService, command: DbCommand) {
    service.dispatch(command).unwrap();
    drain(service);
}
fn query(service: &mut DbService, text: &str) -> Vec<Vec<Value>> {
    send(
        service,
        DbCommand::Execute {
            sql: text.to_owned(),
        },
    );
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    service.result.as_ref().unwrap().rows.clone()
}
fn failed_query(service: &mut DbService, text: &str) {
    send(
        service,
        DbCommand::Execute {
            sql: text.to_owned(),
        },
    );
    assert!(
        service.last_error.is_some(),
        "query unexpectedly accepted: {text}"
    );
    assert!(service.result.is_none());
}

#[test]
fn dynamic_values_remain_typed_and_full_width() {
    let lab = Lab::new();
    let mut service = lab.service();
    let rows = query(
        &mut service,
        "SELECT sample,description,temperature,encoded FROM readings ORDER BY sample",
    );
    assert_eq!(
        rows[0],
        vec![
            Value::Integer(10),
            Value::Text("Medição α".into()),
            Value::Real(-2.25),
            Value::Blob(vec![0xde, 0xad])
        ]
    );
    assert_eq!(
        rows[1],
        vec![
            Value::Integer(20),
            Value::Text(String::new()),
            Value::Null,
            Value::Null
        ]
    );
    assert_eq!(
        query(
            &mut service,
            "SELECT CAST('9223372036854775807' AS INTEGER), CAST('-9223372036854775808' AS INTEGER)"
        )[0],
        vec![Value::Integer(i64::MAX), Value::Integer(i64::MIN)]
    );
    assert_eq!(value_type_name(&rows[0][2]), "REAL");
    assert_eq!(display_value(&rows[1][1]), "\"\"");
    assert_eq!(
        display_value(&Value::Blob(vec![0xde, 0xad])),
        "BLOB[2] X'DEAD'"
    );
    assert!(display_value(&Value::Text("z".repeat(900))).len() < 600);
}

#[test]
fn catalog_matches_sqlite_declared_types_and_nullable_rules() {
    let lab = Lab::new();
    let database = Connection::open(&lab.path).unwrap();
    database.execute_batch("CREATE TABLE \"route\"\"names\" (name TEXT PRIMARY KEY, required TEXT NOT NULL); CREATE VIEW names AS SELECT description FROM readings;").unwrap();
    let mut service = lab.service();
    let readings = service
        .schema
        .iter()
        .find(|table| table.name == "readings")
        .unwrap();
    assert_eq!(readings.columns[0].data_type, "INTEGER");
    assert!(readings.columns[0].primary_key);
    assert!(!readings.columns[0].nullable);
    let named = service
        .schema
        .iter()
        .find(|table| table.name == "route\"names")
        .unwrap();
    assert!(named.columns[0].nullable); // Ordinary SQLite TEXT PRIMARY KEY allows NULL.
    assert!(!named.columns[1].nullable);
    assert!(service.schema.iter().any(|table| table.name == "names"));
    database
        .execute("CREATE TABLE newly_added(x INTEGER)", [])
        .unwrap();
    send(&mut service, DbCommand::RefreshSchema);
    assert!(
        service
            .schema
            .iter()
            .any(|table| table.name == "newly_added")
    );
}

#[test]
fn parser_handles_semicolons_in_text_and_rejects_entire_multi_statement_input() {
    let lab = Lab::new();
    let mut service = lab.service();
    assert_eq!(
        query(&mut service, "SELECT 'α;β' /* ; */; -- trailing ;")[0][0],
        Value::Text("α;β".into())
    );
    failed_query(&mut service, "SELECT 7; SELECT 8;");
    failed_query(
        &mut service,
        "UPDATE readings SET sample=11; SELECT sample FROM readings",
    );
    failed_query(&mut service, "SELECT absent_column FROM readings");
    assert_eq!(
        query(&mut service, "SELECT sample FROM readings ORDER BY sample")[0][0],
        Value::Integer(10)
    );
}

#[test]
fn database_readonly_policy_blocks_file_session_and_temporary_mutations() {
    let lab = Lab::new();
    let mut service = lab.service();
    for text in [
        "DELETE FROM readings",
        "CREATE TEMP TABLE transient(a)",
        "PRAGMA query_only=0",
        "BEGIN",
        "VACUUM",
        "SELECT load_extension('anything')",
    ] {
        failed_query(&mut service, text);
    }
    let forbidden = lab.path.with_file_name("not-created.sqlite");
    failed_query(
        &mut service,
        &format!("ATTACH '{}' AS extra", forbidden.display()),
    );
    assert!(!forbidden.exists());
    let independent = Connection::open(&lab.path).unwrap();
    let count: i64 = independent
        .query_row("SELECT count(*) FROM readings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(query(&mut service, "SELECT 123")[0][0], Value::Integer(123));
}

#[test]
fn failed_connection_does_not_create_files_or_replace_previous_connection() {
    let lab = Lab::new();
    let mut service = lab.service();
    let missing = lab.path.with_file_name("missing.sqlite");
    send(&mut service, DbCommand::Connect(missing.clone()));
    assert!(!missing.exists());
    assert!(service.last_error.is_some());
    assert!(service.connected);
    assert_eq!(service.config.as_deref(), Some(lab.path.as_path()));
    assert_eq!(
        query(
            &mut service,
            "SELECT description FROM readings WHERE sample=10"
        )[0][0],
        Value::Text("Medição α".into())
    );
}

#[test]
fn row_and_cell_limits_do_not_cut_values() {
    let lab = Lab::new();
    let mut service = lab.service();
    query(
        &mut service,
        "WITH RECURSIVE sequence(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM sequence WHERE n<650) SELECT n FROM sequence",
    );
    let result = service.result.as_ref().unwrap();
    assert_eq!(result.rows.len(), QUERY_LIMIT);
    assert_eq!(result.rows.last().unwrap()[0], Value::Integer(499));
    assert!(result.truncated);
    query(&mut service, "SELECT zeroblob(65536)");
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Value::Blob(vec![0; 65536])
    );
    query(&mut service, "SELECT zeroblob(65538)");
    let result = service.result.as_ref().unwrap();
    assert!(result.rows.is_empty());
    assert!(result.truncated);
    assert!(result.limit_reason.as_ref().unwrap().contains("64 KiB"));
}

#[test]
fn accumulated_result_byte_budget_is_enforced() {
    let lab = Lab::new();
    let mut service = lab.service();
    query(
        &mut service,
        "WITH RECURSIVE sequence(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM sequence WHERE n<499) SELECT zeroblob(32768) FROM sequence",
    );
    let result = service.result.as_ref().unwrap();
    assert!(!result.rows.is_empty());
    assert!(result.rows.len() < 128);
    assert!(result.truncated);
    assert!(result.limit_reason.as_ref().unwrap().contains("4 MiB"));
    assert!(
        result
            .rows
            .iter()
            .all(|row| row[0] == Value::Blob(vec![0; 32768]))
    );
}

#[test]
fn invalid_text_encoding_is_a_query_error_not_lossy_text() {
    let lab = Lab::new();
    let mut service = lab.service();
    failed_query(&mut service, "SELECT CAST(X'FF' AS TEXT)");
    assert!(
        service
            .last_error
            .as_ref()
            .unwrap()
            .to_ascii_lowercase()
            .contains("utf")
    );
}

#[test]
fn real_running_sqlite_operation_can_be_interrupted_and_session_remains_usable() {
    let lab = Lab::new();
    let mut service = lab.service();
    service.dispatch(DbCommand::Execute { sql: "WITH RECURSIVE forever(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM forever) SELECT sum(n) FROM forever".into() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !service.is_running() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    thread::sleep(Duration::from_millis(25));
    let started = Instant::now();
    send(&mut service, DbCommand::Cancel);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        service
            .last_error
            .as_ref()
            .unwrap()
            .contains("SQLite interrompido"),
        "{:?}",
        service.last_error
    );
    assert!(service.result.is_none());
    assert_eq!(query(&mut service, "SELECT 987")[0][0], Value::Integer(987));
}

#[test]
fn data_version_proves_connection_is_retained_between_queries() {
    let lab = Lab::new();
    let mut service = lab.service();
    let first = query(&mut service, "SELECT * FROM pragma_data_version")[0][0].clone();
    let external = Connection::open(&lab.path).unwrap();
    external
        .execute("INSERT INTO readings(sample) VALUES (30)", [])
        .unwrap();
    drop(external);
    let second = query(&mut service, "SELECT * FROM pragma_data_version")[0][0].clone();
    assert_ne!(first, second); // Reconnecting would start a new data_version counter.
}

#[test]
fn queued_requests_are_bounded_and_latest_result_wins_then_disconnects() {
    let lab = Lab::new();
    let mut service = lab.service();
    for number in 40..48 {
        service
            .dispatch(DbCommand::Execute {
                sql: format!("SELECT {number}"),
            })
            .unwrap();
    }
    assert!(
        service
            .dispatch(DbCommand::Execute {
                sql: "SELECT 100".into()
            })
            .is_err()
    );
    drain(&mut service);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Value::Integer(47)
    );
    send(&mut service, DbCommand::Disconnect);
    assert!(!service.connected);
    assert!(service.config.is_none());
    assert!(service.schema.is_empty());
    assert!(
        service
            .dispatch(DbCommand::Execute {
                sql: "SELECT 1".into()
            })
            .is_err()
    );
}

fn rejected_catalog(path: &Path) -> String {
    let mut service = DbService::new();
    send(&mut service, DbCommand::Connect(path.into()));
    assert!(!service.connected);
    assert!(service.schema.is_empty());
    service.last_error.take().unwrap()
}

#[test]
fn catalog_table_and_column_limits_are_explicit_errors() {
    let lab = Lab::new();
    let database = Connection::open(&lab.path).unwrap();
    for index in 0..257 {
        database
            .execute(&format!("CREATE TABLE extra_{index}(x)"), [])
            .unwrap();
    }
    assert!(rejected_catalog(&lab.path).contains("256"));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wide.sqlite");
    let database = Connection::open(&path).unwrap();
    let columns = (0..257)
        .map(|i| format!("field_{i}"))
        .collect::<Vec<_>>()
        .join(",");
    database
        .execute(&format!("CREATE TABLE wide({columns})"), [])
        .unwrap();
    assert!(!rejected_catalog(&path).is_empty());
}

#[test]
fn catalog_metadata_budget_prevents_unbounded_names() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("names.sqlite");
    let database = Connection::open(&path).unwrap();
    let long_name = "a".repeat(8192);
    for index in 0..70 {
        database
            .execute(
                &format!("CREATE TABLE t_{index}(\"{long_name}1\",\"{long_name}2\")"),
                [],
            )
            .unwrap();
    }
    assert!(rejected_catalog(&path).contains("metadados"));
}
