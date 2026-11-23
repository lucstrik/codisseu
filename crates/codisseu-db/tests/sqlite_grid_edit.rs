//! Real disposable SQLite tests; independent connections verify persistence and isolation.
use codisseu_db::remote::{Cell, CellEdit, EditBatch};
use codisseu_db::{DbCommand, DbService, Value, sqlite_cell};
use rusqlite::Connection;
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

struct Lab {
    _temp: tempfile::TempDir,
    path: PathBuf,
}
impl Lab {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("grid.sqlite");
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE readings(tenant INTEGER NOT NULL,id INTEGER NOT NULL,name TEXT NOT NULL,quantity INTEGER,enabled BOOLEAN NOT NULL DEFAULT 1,PRIMARY KEY(tenant,id)); INSERT INTO readings VALUES(1,1,'Ada',12,1),(2,1,'Bruno',25,0); CREATE TABLE generated(id INTEGER PRIMARY KEY,a INTEGER,b INTEGER GENERATED ALWAYS AS (a*2) STORED); INSERT INTO generated(id,a) VALUES(1,5); CREATE TABLE ambiguous(id INTEGER,name TEXT); INSERT INTO ambiguous VALUES(1,'same'),(1,'same'); CREATE TABLE nullable(a TEXT,b TEXT,v TEXT,PRIMARY KEY(a,b)); INSERT INTO nullable VALUES(NULL,'b','one'),(NULL,'b','two');").unwrap();
        Self { _temp: temp, path }
    }
    fn service(&self, writable: bool) -> DbService {
        let mut db = DbService::new();
        send(
            &mut db,
            if writable {
                DbCommand::ConnectWritable(self.path.clone())
            } else {
                DbCommand::Connect(self.path.clone())
            },
        );
        assert!(db.connected, "{:?}", db.last_error);
        db
    }
    fn name(&self, tenant: i64) -> String {
        Connection::open(&self.path)
            .unwrap()
            .query_row("SELECT name FROM readings WHERE tenant=?1", [tenant], |r| {
                r.get(0)
            })
            .unwrap()
    }
}
fn send(db: &mut DbService, command: DbCommand) {
    db.dispatch(command).unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    while db.is_busy() {
        db.poll();
        assert!(Instant::now() < deadline, "{}", db.status);
        thread::sleep(Duration::from_millis(1));
    }
}
fn query(db: &mut DbService, sql: &str) {
    send(db, DbCommand::Execute { sql: sql.into() });
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
}
fn batch(db: &DbService, changes: &[(usize, usize, Cell)]) -> EditBatch {
    let (table, columns) = db.origin.clone().expect("editable origin");
    let result = db.result.as_ref().unwrap();
    EditBatch {
        table,
        columns,
        edits: changes
            .iter()
            .map(|(row, col, value)| CellEdit {
                row: result.rows[*row].iter().map(sqlite_cell).collect(),
                column: result.columns[*col].clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

#[test]
fn staged_multiple_cells_share_update_manual_rollback_and_commit_isolate_rows() {
    let lab = Lab::new();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM readings ORDER BY tenant");
    let edits = batch(
        &db,
        &[
            (0, 2, Cell::Text("Ada edited".into())),
            (0, 3, Cell::Number("99".into())),
        ],
    );
    send(&mut db, DbCommand::Apply(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert!(db.transaction);
    assert_eq!(db.apply_revision, 1);
    assert_eq!(
        db.result.as_ref().unwrap().rows[0][2],
        Value::Text("Ada edited".into())
    );
    assert_eq!(lab.name(1), "Ada");
    assert_eq!(lab.name(2), "Bruno");
    assert!(db.dispatch(DbCommand::Disconnect).is_err());
    send(&mut db, DbCommand::Rollback);
    assert!(!db.transaction);
    assert_eq!(lab.name(1), "Ada");
    let edits = batch(
        &db,
        &[
            (0, 2, Cell::Text("Ada committed".into())),
            (0, 3, Cell::Null),
        ],
    );
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert!(!db.transaction);
    assert_eq!(lab.name(1), "Ada committed");
    assert_eq!(lab.name(2), "Bruno");
    assert_eq!(db.result.as_ref().unwrap().rows[0][3], Value::Null);
}

#[test]
fn concurrent_conflict_reverts_whole_batch_and_preserves_competitor() {
    let lab = Lab::new();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM readings ORDER BY tenant");
    let edits = batch(
        &db,
        &[
            (0, 2, Cell::Text("lost".into())),
            (1, 2, Cell::Text("also lost".into())),
        ],
    );
    Connection::open(&lab.path)
        .unwrap()
        .execute("UPDATE readings SET quantity=26 WHERE tenant=2", [])
        .unwrap();
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(
        db.last_error.as_deref().unwrap_or("").contains("Conflito"),
        "{:?}",
        db.last_error
    );
    assert!(!db.transaction);
    assert_eq!(db.apply_revision, 0);
    assert_eq!(lab.name(1), "Ada");
    assert_eq!(lab.name(2), "Bruno");
}

#[test]
fn schema_null_numeric_keys_and_forged_original_values_are_validated() {
    let lab = Lab::new();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM readings ORDER BY tenant");
    for edits in [
        batch(&db, &[(0, 3, Cell::Text("twelve".into()))]),
        batch(&db, &[(0, 3, Cell::Number("1.25".into()))]),
        batch(&db, &[(0, 2, Cell::Null)]),
        batch(&db, &[(0, 0, Cell::Number("2".into()))]),
    ] {
        send(&mut db, DbCommand::ApplyAndCommit(edits));
        assert!(db.last_error.is_some());
        assert!(!db.transaction);
        assert_eq!(lab.name(1), "Ada");
    }
    let mut forged = batch(&db, &[(0, 2, Cell::Text("forged".into()))]);
    forged.edits[0].row[0] = Cell::Number("999".into());
    send(&mut db, DbCommand::ApplyAndCommit(forged));
    assert!(db.last_error.is_some());
    let edits = batch(&db, &[(0, 2, Cell::Text("stale".into()))]);
    Connection::open(&lab.path)
        .unwrap()
        .execute("ALTER TABLE readings ADD COLUMN extra TEXT", [])
        .unwrap();
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_some());
    assert_eq!(lab.name(1), "Ada");
}

#[test]
fn projection_reorder_quote_and_embedded_sql_value_are_safe() {
    let lab = Lab::new();
    let mut db = lab.service(true);
    query(
        &mut db,
        "SELECT name,id,tenant FROM readings WHERE tenant=1 ORDER BY id",
    );
    assert!(db.origin.is_some());
    let hostile = "'; DELETE FROM readings; -- 🦀";
    let edits = batch(&db, &[(0, 0, Cell::Text(hostile.into()))]);
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert_eq!(lab.name(1), hostile);
    assert_eq!(lab.name(2), "Bruno");
}

#[test]
fn ambiguous_queries_nullable_identity_and_generated_cells_stay_protected() {
    let lab = Lab::new();
    let mut db = lab.service(true);
    for sql in [
        "SELECT * FROM ambiguous",
        "SELECT * FROM nullable",
        "SELECT name FROM readings",
        "SELECT DISTINCT * FROM readings",
        "SELECT a.* FROM readings a JOIN readings b ON a.id=b.id",
        "SELECT tenant,id,name||'x' AS name FROM readings",
        "SELECT tenant,id,name AS renamed FROM readings",
        "WITH q AS (SELECT * FROM readings) SELECT * FROM q",
        "SELECT tenant,id,name FROM readings UNION SELECT tenant,id,name FROM readings",
    ] {
        query(&mut db, sql);
        assert!(db.origin.is_none(), "unsafe editable query: {sql}");
    }
    query(&mut db, "SELECT * FROM generated");
    assert!(
        db.origin.as_ref().unwrap().1[2]
            .data_type
            .starts_with("generated ")
    );
    let edits = batch(&db, &[(0, 2, Cell::Number("99".into()))]);
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_some());
    let edits = batch(&db, &[(0, 1, Cell::Number("7".into()))]);
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert_eq!(db.result.as_ref().unwrap().rows[0][2], Value::Integer(14));
}

#[test]
fn writable_opt_in_does_not_allow_arbitrary_sql_writes() {
    let lab = Lab::new();
    let mut db = lab.service(false);
    query(&mut db, "SELECT * FROM readings");
    assert!(!db.writable);
    assert!(db.origin.is_none());
    assert!(db.dispatch(DbCommand::Commit).is_err());
    send(&mut db, DbCommand::ConnectWritable(lab.path.clone()));
    assert!(db.writable);
    for sql in [
        "UPDATE readings SET name='bad' RETURNING *",
        "BEGIN",
        "PRAGMA query_only=0",
        "ATTACH ':memory:' AS other",
        "DELETE FROM readings",
    ] {
        send(&mut db, DbCommand::Execute { sql: sql.into() });
        assert!(db.last_error.is_some(), "unsafe accepted: {sql}");
    }
    assert_eq!(lab.name(1), "Ada");
    assert!(!db.transaction);
}

#[test]
fn quoted_identifiers_full_width_integer_and_trigger_writes_are_guarded() {
    let lab = Lab::new();
    Connection::open(&lab.path).unwrap().execute_batch("CREATE TABLE \"odd\"\"table\"(\"key\"\"name\" INTEGER PRIMARY KEY,\"value\"\"name\" INTEGER,txt TEXT); INSERT INTO \"odd\"\"table\" VALUES(1,1,'ok');").unwrap();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM \"odd\"\"table\"");
    let edits = batch(
        &db,
        &[
            (0, 1, Cell::Number(i64::MAX.to_string())),
            (0, 2, Cell::Text("safe ' ? ;".into())),
        ],
    );
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert_eq!(
        db.result.as_ref().unwrap().rows[0][1],
        Value::Integer(i64::MAX)
    );
    Connection::open(&lab.path).unwrap().execute_batch("CREATE TRIGGER side_effect AFTER UPDATE ON readings BEGIN UPDATE readings SET name='unexpected' WHERE tenant=2; END;").unwrap();
    query(&mut db, "SELECT * FROM readings ORDER BY tenant");
    let edits = batch(&db, &[(0, 2, Cell::Text("should rollback".into()))]);
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_some());
    assert_eq!(lab.name(1), "Ada");
    assert_eq!(lab.name(2), "Bruno");
    assert!(!db.transaction);
}

#[test]
fn explicit_commit_preserves_prior_submitted_edits_and_next_failed_batch() {
    let lab = Lab::new();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM readings ORDER BY tenant");
    let edits = batch(&db, &[(0, 2, Cell::Text("first submitted".into()))]);
    send(&mut db, DbCommand::Apply(edits));
    assert!(db.last_error.is_none());
    assert!(db.transaction);
    let edits = batch(&db, &[(1, 2, Cell::Text("second submitted".into()))]);
    send(&mut db, DbCommand::Apply(edits));
    assert!(db.last_error.is_none());
    assert!(db.transaction);
    // A rejected NULL never removes earlier successful submissions in this transaction.
    let edits = batch(&db, &[(0, 2, Cell::Null)]);
    send(&mut db, DbCommand::Apply(edits));
    assert!(db.last_error.is_some());
    assert!(db.transaction);
    assert_eq!(lab.name(1), "Ada");
    assert_eq!(lab.name(2), "Bruno");
    send(&mut db, DbCommand::Commit);
    assert!(db.last_error.is_none());
    assert!(!db.transaction);
    assert_eq!(lab.name(1), "first submitted");
    assert_eq!(lab.name(2), "second submitted");
}

#[test]
fn sqlite_rollback_constraint_explicitly_reports_loss_of_prior_uncommitted_batch() {
    let lab = Lab::new();
    Connection::open(&lab.path).unwrap().execute_batch("CREATE TABLE rollback_constraint(id INTEGER PRIMARY KEY,name TEXT UNIQUE ON CONFLICT ROLLBACK); INSERT INTO rollback_constraint VALUES(1,'A'),(2,'B');").unwrap();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM rollback_constraint ORDER BY id");
    let edits = batch(&db, &[(0, 1, Cell::Text("submitted".into()))]);
    send(&mut db, DbCommand::Apply(edits));
    assert!(db.last_error.is_none());
    assert!(db.transaction);
    let edits = batch(&db, &[(1, 1, Cell::Text("submitted".into()))]);
    send(&mut db, DbCommand::Apply(edits));
    assert!(
        db.last_error
            .as_deref()
            .unwrap_or("")
            .contains("reverteu toda a transação"),
        "{:?}",
        db.last_error
    );
    assert!(!db.transaction);
    let values: Vec<String> = Connection::open(&lab.path)
        .unwrap()
        .prepare("SELECT name FROM rollback_constraint ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(values, ["A", "B"]);
}

#[test]
fn sorted_page_retains_original_identity_and_refreshes_only_its_own_view() {
    use codisseu_db::query_page::{ColumnSort, QueryPageRequest};
    let lab = Lab::new();
    let mut db = lab.service(true);
    let page = QueryPageRequest {
        sql: "SELECT name,id,tenant FROM readings ORDER BY tenant".into(),
        offset: 1,
        sort: Some(ColumnSort {
            column: 2,
            descending: true,
        }),
    };
    send(&mut db, DbCommand::ExecutePage(page));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert_eq!(db.result.as_ref().unwrap().rows.len(), 1);
    assert_eq!(
        db.result.as_ref().unwrap().rows[0][0],
        Value::Text("Ada".into())
    );
    let edits = batch(&db, &[(0, 0, Cell::Text("Ada from page".into()))]);
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert!(!db.transaction);
    assert_eq!(lab.name(1), "Ada from page");
    assert_eq!(lab.name(2), "Bruno");
    assert_eq!(db.result.as_ref().unwrap().rows.len(), 1);
    assert_eq!(
        db.result.as_ref().unwrap().rows[0][0],
        Value::Text("Ada from page".into())
    );
    assert!(db.origin.is_some());
    send(
        &mut db,
        DbCommand::ExecutePage(QueryPageRequest::first(
            "SELECT a.* FROM readings a JOIN readings b ON a.id=b.id".into(),
        )),
    );
    assert!(db.last_error.is_none());
    assert!(db.origin.is_none());
}

#[test]
fn failed_deferred_commit_reports_whole_transaction_rollback_without_false_success() {
    let lab = Lab::new();
    Connection::open(&lab.path).unwrap().execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE parents(id INTEGER PRIMARY KEY); INSERT INTO parents VALUES(1); CREATE TABLE children(id INTEGER PRIMARY KEY,parent_id INTEGER NOT NULL REFERENCES parents(id) DEFERRABLE INITIALLY DEFERRED,name TEXT NOT NULL); INSERT INTO children VALUES(1,1,'original');").unwrap();
    let mut db = lab.service(true);
    query(&mut db, "SELECT * FROM children");
    let edits = batch(&db, &[(0, 2, Cell::Text("previous submit".into()))]);
    send(&mut db, DbCommand::Apply(edits));
    assert!(db.last_error.is_none(), "{:?}", db.last_error);
    assert!(db.transaction);
    let revision = db.apply_revision;
    let edits = batch(&db, &[(0, 1, Cell::Number("999".into()))]);
    send(&mut db, DbCommand::ApplyAndCommit(edits));
    assert!(
        db.last_error
            .as_deref()
            .unwrap_or("")
            .contains("toda a transação foi revertida"),
        "{:?}",
        db.last_error
    );
    assert!(db.status.contains("inclusive os envios anteriores"));
    assert!(!db.status.contains("Commit concluído"));
    assert!(!db.transaction);
    assert_eq!(db.apply_revision, revision);
    let observer = Connection::open(&lab.path).unwrap();
    let record: (i64, String) = observer
        .query_row("SELECT parent_id,name FROM children WHERE id=1", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(record, (1, "original".into()));
}
