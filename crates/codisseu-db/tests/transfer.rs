use codisseu_db::{
    DbCommand, DbService,
    remote::{Cell, TableRef},
    transfer::*,
};
use std::time::{Duration, Instant};
fn data() -> TransferData {
    TransferData {
        columns: vec!["id".into(), "text".into(), "nil".into(), "binary".into()],
        rows: vec![
            vec![
                Cell::Number("900719925474099312345.0000000100".into()),
                Cell::Text("a,\"b\"\r\n\\N".into()),
                Cell::Null,
                Cell::Binary(vec![0, 255, 10]),
            ],
            vec![
                Cell::Bool(true),
                Cell::Text("\\N".into()),
                Cell::Text("".into()),
                Cell::Json("{\"x\":9007199254740993123}".into()),
            ],
        ],
        source_truncated: true,
    }
}
#[test]
fn tagged_roundtrip_preserves_every_cell_and_truncation() {
    for format in [TransferFormat::CodisseuJson, TransferFormat::CodisseuNdjson] {
        let options = TransferOptions {
            format,
            ..Default::default()
        };
        let encoded = encode(&options, &data()).unwrap();
        assert_eq!(
            decode(&options, std::str::from_utf8(&encoded).unwrap()).unwrap(),
            data()
        );
    }
}
#[test]
fn csv_null_literal_quotes_newlines_and_custom_delimiter() {
    let input = TransferData {
        columns: vec!["a".into(), "b".into(), "c".into()],
        rows: vec![
            vec![
                Cell::Null,
                Cell::Text("\\N".into()),
                Cell::Text("a;\"b\"\r\nc".into()),
            ],
            vec![
                Cell::Text("".into()),
                Cell::Text("\\path".into()),
                Cell::Text("=SUM(A1)".into()),
            ],
        ],
        source_truncated: false,
    };
    let options = TransferOptions {
        delimiter: Some(b';'),
        ..Default::default()
    };
    let bytes = encode(&options, &input).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("\\N;\"\\N\""));
    assert_eq!(decode(&options, &text).unwrap(), input);
    assert!(decode(&options, "a;b\n\"unterminated").is_err());
    assert!(decode(&options, "a;b\n\"close\"oops;b").is_err());
}
#[test]
fn standard_json_exact_numbers_and_structural_values() {
    let options = TransferOptions {
        format: TransferFormat::Json,
        ..Default::default()
    };
    let input = "[{\"n\":900719925474099312345.0000000100,\"j\":{\"n\":900719925474099312345},\"nil\":null}]";
    let decoded = decode(&options, input).unwrap();
    assert_eq!(
        decoded.rows[0][1],
        Cell::Number("900719925474099312345.0000000100".into())
    );
    let output = String::from_utf8(encode(&options, &decoded).unwrap()).unwrap();
    assert!(output.contains("900719925474099312345.0000000100"));
    assert!(output.contains("{\"n\":900719925474099312345}"));
    assert!(decode(&options, "[{\"x\":1},{\"y\":2}]").is_err());
}
#[test]
fn budgets_and_sql_export_are_explicit() {
    let mut source = data();
    source.rows = vec![vec![Cell::Text("x".repeat(MAX_TRANSFER_CELL + 1)); 4]];
    assert!(source.validate().is_err());
    let options = TransferOptions {
        format: TransferFormat::SqlInsert,
        sql_table: Some(TableRef {
            namespace: "public".into(),
            name: "ta\"ble".into(),
        }),
        ..Default::default()
    };
    let sql = String::from_utf8(encode(&options, &data()).unwrap()).unwrap();
    assert!(sql.contains("ATENÇÃO: origem truncada"));
    assert!(sql.contains("\"ta\"\"ble\""));
    assert!(decode(&options, &sql).is_err());
    let spreadsheet = TransferOptions {
        spreadsheet_safe: true,
        ..Default::default()
    };
    let single = TransferData {
        columns: vec!["x".into()],
        rows: vec![vec![Cell::Text("=1+2".into())]],
        source_truncated: false,
    };
    assert!(
        String::from_utf8(encode(&spreadsheet, &single).unwrap())
            .unwrap()
            .contains("'=1+2")
    );
}
fn wait(db: &mut DbService) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while db.is_busy() {
        db.poll();
        assert!(Instant::now() < deadline, "{}", db.status);
        std::thread::sleep(Duration::from_millis(1));
    }
    db.poll();
}
fn request(rows: Vec<Vec<Cell>>) -> ImportRequest {
    ImportRequest {
        allow_partial: false,
        target: ImportTarget::SqlTable(TableRef {
            namespace: "main".into(),
            name: "target".into(),
        }),
        data: TransferData {
            columns: vec!["id".into(), "name".into()],
            rows,
            source_truncated: false,
        },
        mapping: vec![
            ImportMapping {
                source_column: 0,
                target_column: "id".into(),
            },
            ImportMapping {
                source_column: 1,
                target_column: "name".into(),
            },
        ],
    }
}
#[test]
fn sqlite_preview_bound_import_atomic_conflict_and_readonly_restored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.sqlite");
    let observer = rusqlite::Connection::open(&path).unwrap();
    observer.execute_batch("CREATE TABLE target(id INTEGER PRIMARY KEY,name TEXT NOT NULL); INSERT INTO target VALUES(1,'existing');").unwrap();
    let mut db = DbService::new();
    db.dispatch(DbCommand::Connect(path.clone())).unwrap();
    wait(&mut db);
    let rows = vec![vec![
        Cell::Number("2".into()),
        Cell::Text("Robert'); DROP TABLE target;--".into()),
    ]];
    db.dispatch(DbCommand::PreviewFileImport {
        path: path.clone(),
        request: request(rows),
    })
    .unwrap();
    wait(&mut db);
    assert!(db.last_error.is_none(), "{}", db.status);
    assert_eq!(
        observer
            .query_row("SELECT count(*) FROM target", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let token = db.import_preview.as_ref().unwrap().preview_id;
    db.dispatch(DbCommand::ImportFileBatch {
        path: path.clone(),
        preview_id: token,
    })
    .unwrap();
    wait(&mut db);
    assert_eq!(
        db.import_report.as_ref().unwrap().applied,
        1,
        "{}",
        db.status
    );
    assert!(!db.import_report.as_ref().unwrap().transaction_pending);
    assert_eq!(
        observer
            .query_row("SELECT name FROM target WHERE id=2", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "Robert'); DROP TABLE target;--"
    );
    let rows = vec![
        vec![
            Cell::Number("3".into()),
            Cell::Text("must roll back".into()),
        ],
        vec![Cell::Number("1".into()), Cell::Text("conflict".into())],
    ];
    db.dispatch(DbCommand::PreviewFileImport {
        path: path.clone(),
        request: request(rows),
    })
    .unwrap();
    wait(&mut db);
    let token = db.import_preview.as_ref().unwrap().preview_id;
    db.dispatch(DbCommand::ImportFileBatch {
        path: path.clone(),
        preview_id: token,
    })
    .unwrap();
    wait(&mut db);
    assert_eq!(db.import_report.as_ref().unwrap().applied, 0);
    assert_eq!(
        observer
            .query_row("SELECT count(*) FROM target WHERE id=3", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.dispatch(DbCommand::Execute {
        sql: "INSERT INTO target VALUES(4,'no') RETURNING id".into(),
    })
    .unwrap();
    wait(&mut db);
    assert!(db.last_error.is_some());
    db.dispatch(DbCommand::Execute {
        sql: "SELECT count(*) FROM target".into(),
    })
    .unwrap();
    wait(&mut db);
    assert!(db.last_error.is_none(), "{}", db.status);
    assert_eq!(db.result.as_ref().unwrap().rows.len(), 1);
}
fn file_wait(service: &mut FileService) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    service.poll();
}
#[test]
fn file_worker_atomic_no_overwrite_render_and_bounded_directory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data.json");
    let mut worker = FileService::new();
    let options = TransferOptions {
        format: TransferFormat::CodisseuJson,
        ..Default::default()
    };
    worker
        .dispatch(FileCommand::Write {
            path: path.clone(),
            options: options.clone(),
            data: data(),
            overwrite: false,
        })
        .unwrap();
    file_wait(&mut worker);
    assert!(worker.error.is_none());
    let original = std::fs::read(&path).unwrap();
    worker
        .dispatch(FileCommand::WriteRaw {
            path: path.clone(),
            text: "replace".into(),
            overwrite: false,
        })
        .unwrap();
    file_wait(&mut worker);
    assert!(worker.error.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    worker
        .dispatch(FileCommand::Read {
            path,
            options: options.clone(),
        })
        .unwrap();
    file_wait(&mut worker);
    assert_eq!(worker.data.as_ref().unwrap(), &data());
    worker
        .dispatch(FileCommand::Render {
            options,
            data: data(),
        })
        .unwrap();
    file_wait(&mut worker);
    assert_eq!(worker.rendered.as_ref().unwrap(), &original);
    for i in 0..260 {
        std::fs::write(directory.path().join(format!("f{i}")), "").unwrap();
    }
    worker
        .dispatch(FileCommand::ListDirectory {
            path: directory.path().to_owned(),
        })
        .unwrap();
    file_wait(&mut worker);
    assert_eq!(worker.directory.as_ref().unwrap().entries.len(), 256);
    assert!(worker.directory.as_ref().unwrap().truncated);
}

#[test]
fn sqlite_late_cancel_keeps_confirmed_commit_ack_and_idle_cancel_invalidates_preview() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancel.sqlite");
    let observer = rusqlite::Connection::open(&path).unwrap();
    observer
        .execute_batch("CREATE TABLE target(id INTEGER PRIMARY KEY,name TEXT NOT NULL)")
        .unwrap();
    let mut db = DbService::new();
    db.dispatch(DbCommand::Connect(path.clone())).unwrap();
    wait(&mut db);
    let rows = vec![vec![
        Cell::Number("1".into()),
        Cell::Text("persisted".into()),
    ]];
    db.dispatch(DbCommand::PreviewFileImport {
        path: path.clone(),
        request: request(rows.clone()),
    })
    .unwrap();
    wait(&mut db);
    let token = db.import_preview.as_ref().unwrap().preview_id;
    db.dispatch(DbCommand::Cancel).unwrap();
    db.dispatch(DbCommand::ImportFileBatch {
        path: path.clone(),
        preview_id: token,
    })
    .unwrap();
    wait(&mut db);
    assert!(
        db.last_error.is_some(),
        "idle cancel must invalidate prepared token"
    );
    assert_eq!(
        observer
            .query_row("SELECT count(*) FROM target", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.dispatch(DbCommand::PreviewFileImport {
        path: path.clone(),
        request: request(rows),
    })
    .unwrap();
    wait(&mut db);
    let token = db.import_preview.as_ref().unwrap().preview_id;
    db.dispatch(DbCommand::ImportFileBatch {
        path: path.clone(),
        preview_id: token,
    })
    .unwrap();
    // Observe commit before polling the UI acknowledgement, then cancel during that gap.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let count: i64 = observer
            .query_row("SELECT count(*) FROM target", [], |r| r.get(0))
            .unwrap();
        if count == 1 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    db.dispatch(DbCommand::Cancel).unwrap();
    wait(&mut db);
    assert_eq!(
        db.import_report.as_ref().unwrap().applied,
        1,
        "late cancel must publish actual commit ACK"
    );
    let rows = vec![vec![
        Cell::Text("900719925474099312345".into()),
        Cell::Text("would lose integer precision".into()),
    ]];
    db.dispatch(DbCommand::PreviewFileImport {
        path,
        request: request(rows),
    })
    .unwrap();
    wait(&mut db);
    assert!(db.last_error.is_some());
    assert!(db.import_preview.is_none());
}
#[test]
fn mapping_partial_snapshot_requires_explicit_opt_in() {
    let mut source = request(vec![vec![
        Cell::Number("1".into()),
        Cell::Text("page".into()),
    ]]);
    source.data.source_truncated = true;
    assert!(map_import(&source).is_err());
    source.allow_partial = true;
    assert_eq!(map_import(&source).unwrap().rows.len(), 1);
}

#[test]
fn parsers_reject_duplicate_json_and_structural_overflow_early() {
    let json = TransferOptions {
        format: TransferFormat::Json,
        ..Default::default()
    };
    assert!(decode(&json, "[{\"x\":1,\"x\":2}]").is_err());
    let too_many = format!(
        "[{}]",
        std::iter::repeat_n("{}", MAX_TRANSFER_ROWS + 1)
            .collect::<Vec<_>>()
            .join(",")
    );
    assert!(decode(&json, &too_many).is_err());
    let mut source = data();
    source.rows = vec![vec![Cell::Null; MAX_TRANSFER_COLUMNS + 1]];
    source.columns = (0..=MAX_TRANSFER_COLUMNS).map(|n| n.to_string()).collect();
    let encoded =
        serde_json::to_string(&serde_json::json!({"codisseu_transfer":1,"data":source})).unwrap();
    let tagged = TransferOptions {
        format: TransferFormat::CodisseuJson,
        ..Default::default()
    };
    assert!(decode(&tagged, &encoded).is_err());
}
