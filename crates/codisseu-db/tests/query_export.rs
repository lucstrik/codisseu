//! Real SQLite streaming and filesystem integrity; generated data is RAII temporary.
use codisseu_db::{
    DbCommand, DbService, Value,
    query_export::{CsvExportOptions, QueryExportRequest},
};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn drain(service: &mut DbService) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline, "{}", service.status);
        thread::sleep(Duration::from_millis(1));
    }
}
fn service(path: &Path) -> DbService {
    drop(rusqlite::Connection::open(path).unwrap());
    let mut service = DbService::new();
    service.dispatch(DbCommand::Connect(path.into())).unwrap();
    drain(&mut service);
    assert!(service.connected, "{}", service.status);
    service
}
fn request(path: PathBuf, sql: &str) -> QueryExportRequest {
    QueryExportRequest {
        path,
        sql: sql.into(),
        options: CsvExportOptions::default(),
        timeout_secs: 30,
    }
}
fn export(service: &mut DbService, request: QueryExportRequest) {
    service.dispatch(DbCommand::ExportQuery(request)).unwrap();
    drain(service);
}
fn only_db(directory: &Path) {
    assert_eq!(
        std::fs::read_dir(directory).unwrap().count(),
        1,
        "temporary/output files survived"
    );
}

#[test]
fn sqlite_streams_one_million_rows_without_changing_preview() {
    let directory = tempfile::tempdir().unwrap();
    let database_path = directory.path().join("source.sqlite");
    let seed = rusqlite::Connection::open(&database_path).unwrap();
    seed.execute_batch("CREATE TABLE entries(id INTEGER PRIMARY KEY,label TEXT); WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<1000000) INSERT INTO entries SELECT n,'ação' FROM seq;").unwrap();
    drop(seed);
    let mut service = service(&database_path);
    service
        .dispatch(DbCommand::Execute {
            sql: "SELECT 77 AS preview".into(),
        })
        .unwrap();
    drain(&mut service);
    let revision = service.result_revision();
    let path = directory.path().join("million.csv");
    let started = Instant::now();
    export(
        &mut service,
        request(
            path.clone(),
            "SELECT id AS n,label FROM entries ORDER BY id",
        ),
    );
    assert!(service.last_error.is_none(), "{}", service.status);
    let report = service.export_report.as_ref().unwrap();
    assert_eq!(report.rows, 1_000_000);
    assert_eq!(report.bytes, std::fs::metadata(&path).unwrap().len());
    assert_eq!(service.export_progress.snapshot().rows, 1_000_000);
    assert_eq!(service.result_revision(), revision);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Value::Integer(77)
    );
    let mut lines = BufReader::new(std::fs::File::open(&path).unwrap()).lines();
    assert_eq!(lines.next().unwrap().unwrap(), "n,label");
    for n in 1..=1_000_000 {
        assert_eq!(lines.next().unwrap().unwrap(), format!("{n},ação"));
    }
    assert!(lines.next().is_none());
    eprintln!(
        "SQLite CSV: 1000000 rows, {} bytes, backend {} ms, wall {} ms",
        report.bytes,
        report.elapsed_ms,
        started.elapsed().as_millis()
    );
    std::fs::remove_file(path).unwrap();
    drop(service);
    only_db(directory.path());
}

#[test]
fn sqlite_csv_preserves_null_empty_marker_quotes_unicode_and_explicit_limit() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = service(&directory.path().join("source.sqlite"));
    let path = directory.path().join("edge.csv");
    export(
        &mut service,
        request(
            path.clone(),
            r#"SELECT NULL AS a, '' AS b, '\N' AS c, 'a,"b' AS d, 'line'||char(10)||'ação' AS e, -12 AS f LIMIT 1"#,
        ),
    );
    assert!(service.last_error.is_none(), "{}", service.status);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "a,b,c,d,e,f\r\n\\N,,\"\\N\",\"a,\"\"b\",\"line\nação\",-12\r\n"
    );
    let mut r = request(
        directory.path().join("safe.csv"),
        "SELECT '=2+2' AS a, -12 AS b",
    );
    r.options.spreadsheet_safe = true;
    r.options.header = false;
    r.options.delimiter = b';';
    export(&mut service, r);
    assert!(service.last_error.is_none(), "{}", service.status);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("safe.csv")).unwrap(),
        "\"'=2+2\";-12\r\n"
    );
}

#[test]
fn sqlite_export_rejects_write_script_large_cells_and_existing_destinations() {
    let directory = tempfile::tempdir().unwrap();
    let db = directory.path().join("source.sqlite");
    let mut service = service(&db);
    for (i, sql) in [
        "CREATE TABLE forbidden(n)",
        "SELECT 1; SELECT 2",
        "SELECT load_extension('absent')",
        "SELECT printf('%.*c',65537,'x')",
        "PRAGMA user_version=9",
    ]
    .iter()
    .enumerate()
    {
        let path = directory.path().join(format!("denied-{i}.csv"));
        export(&mut service, request(path.clone(), sql));
        assert!(service.last_error.is_some(), "accepted: {sql}");
        assert!(!path.exists());
        only_db(directory.path());
    }
    let path = directory.path().join("existing.csv");
    std::fs::write(&path, b"preserve").unwrap();
    export(&mut service, request(path.clone(), "SELECT 1"));
    assert!(service.last_error.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), b"preserve");
    let check = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        check
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn cancelling_full_query_removes_partial_file_and_preserves_preview() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = service(&directory.path().join("source.sqlite"));
    service
        .dispatch(DbCommand::Execute {
            sql: "SELECT 88".into(),
        })
        .unwrap();
    drain(&mut service);
    let revision = service.result_revision();
    let path = directory.path().join("cancel.csv");
    service.dispatch(DbCommand::ExportQuery(request(path.clone(),"WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<1000000000) SELECT n FROM seq"))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.export_progress.snapshot().rows < 1024 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    service.dispatch(DbCommand::Cancel).unwrap();
    drain(&mut service);
    assert!(service.export_report.is_none());
    assert!(service.last_error.is_some());
    assert!(!path.exists());
    only_db(directory.path());
    assert_eq!(service.result_revision(), revision);
    service
        .dispatch(DbCommand::Execute {
            sql: "SELECT 99".into(),
        })
        .unwrap();
    drain(&mut service);
    assert!(service.last_error.is_none());
}

#[test]
fn export_request_rejects_ambiguous_null_and_invalid_configuration() {
    let mut r = request("a.csv".into(), "SELECT 1");
    for marker in ["", ",", "a\n", "\""] {
        r.options.null_marker = marker.into();
        assert!(r.validate().is_err());
    }
    r.options.null_marker = "NULL".into();
    r.timeout_secs = 3601;
    assert!(r.validate().is_err());
    r.timeout_secs = 30;
    r.options.delimiter = b'"';
    assert!(r.validate().is_err());
}

#[test]
fn sqlite_export_timeout_removes_partial_output() {
    let directory = tempfile::tempdir().unwrap();
    let mut service = service(&directory.path().join("source.sqlite"));
    let path = directory.path().join("timeout.csv");
    let mut r = request(
        path.clone(),
        "WITH RECURSIVE seq(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM seq WHERE n<1000000000) SELECT n FROM seq",
    );
    r.timeout_secs = 1;
    export(&mut service, r);
    assert!(service.export_report.is_none());
    assert!(
        service
            .last_error
            .as_ref()
            .is_some_and(|s| s.contains("Prazo")),
        "{}",
        service.status
    );
    assert!(!path.exists());
    only_db(directory.path());
}
