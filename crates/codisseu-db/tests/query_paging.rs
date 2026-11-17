//! Real SQLite queries: bounded pages and server ordering retain the original
//! query's scope, read-only authorizer and cancellation/session contract.
use codisseu_db::{
    DbCommand, DbService, QUERY_LIMIT, Value,
    query_page::{ColumnSort, QueryPageRequest},
};
use rusqlite::Connection;
use std::{
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
    db: DbService,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pages.sqlite");
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch(
                "CREATE TABLE samples(id INTEGER PRIMARY KEY, label TEXT); \
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<1205) \
             INSERT INTO samples SELECT i, printf('sample-%04d',i) FROM n;",
            )
            .unwrap();
        drop(writer);
        let mut db = DbService::new();
        db.dispatch(DbCommand::Connect(path.clone())).unwrap();
        drain(&mut db);
        assert!(db.connected, "{:?}", db.last_error);
        Self {
            _directory: directory,
            path,
            db,
        }
    }

    fn page(&mut self, sql: &str, offset: u64, sort: Option<ColumnSort>) -> Vec<Vec<Value>> {
        self.db
            .dispatch(DbCommand::ExecutePage(QueryPageRequest {
                sql: sql.into(),
                offset,
                sort,
            }))
            .unwrap();
        drain(&mut self.db);
        assert!(self.db.last_error.is_none(), "{:?}", self.db.last_error);
        self.db.result.as_ref().unwrap().rows.clone()
    }
}

fn drain(db: &mut DbService) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while db.is_busy() {
        db.poll();
        assert!(Instant::now() < deadline, "{}", db.status);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn bounded_pages_cover_every_row_and_last_page_without_counting() {
    let mut fixture = Fixture::new();
    let mut all = Vec::new();
    for (offset, expected, more) in [(0, QUERY_LIMIT, true), (500, 500, true), (1000, 205, false)] {
        let rows = fixture.page("SELECT id FROM samples ORDER BY id", offset, None);
        assert_eq!(rows.len(), expected);
        assert_eq!(fixture.db.result.as_ref().unwrap().truncated, more);
        all.extend(rows.into_iter().map(|row| row[0].clone()));
    }
    assert_eq!(all, (1..=1205).map(Value::Integer).collect::<Vec<_>>());
    assert!(
        fixture
            .page("SELECT id FROM samples ORDER BY id", 1500, None)
            .is_empty()
    );
    assert!(!fixture.db.result.as_ref().unwrap().truncated);
}

#[test]
fn column_order_is_applied_to_the_complete_query_before_page_limit() {
    let mut fixture = Fixture::new();
    let sql = "SELECT id AS duplicate, label AS duplicate FROM samples ORDER BY id";
    let descending = Some(ColumnSort {
        column: 0,
        descending: true,
    });
    let rows = fixture.page(sql, 0, descending);
    assert_eq!(
        fixture.db.result.as_ref().unwrap().columns,
        ["duplicate", "duplicate"]
    );
    assert_eq!(rows[0][0], Value::Integer(1205));
    assert_eq!(rows[499][0], Value::Integer(706));
    assert_eq!(
        fixture.page(sql, 500, descending)[0][0],
        Value::Integer(705)
    );
    let ascending = Some(ColumnSort {
        column: 1,
        descending: false,
    });
    assert_eq!(
        fixture.page(sql, 1000, ascending)[0][1],
        Value::Text("sample-1001".into())
    );
    // A sort is presentation state; no mutation of the original result query.
    assert_eq!(fixture.page(sql, 0, None)[0][0], Value::Integer(1));
}

#[test]
fn original_limit_cte_union_and_quoted_delimiters_keep_their_semantics() {
    let mut fixture = Fixture::new();
    let sql = "-- comment ;\nWITH subset AS (SELECT id FROM samples ORDER BY id LIMIT 600 OFFSET 100) SELECT id, 'a;b' FROM subset; -- trailing ;";
    let rows = fixture.page(sql, 500, None);
    assert_eq!(rows.len(), 100);
    assert_eq!(
        rows[0],
        vec![Value::Integer(601), Value::Text("a;b".into())]
    );
    assert!(!fixture.db.result.as_ref().unwrap().truncated);
    let rows = fixture.page(
        "SELECT 9 UNION ALL SELECT 1 -- terminal comment",
        0,
        Some(ColumnSort {
            column: 0,
            descending: false,
        }),
    );
    assert_eq!(rows, vec![vec![Value::Integer(1)], vec![Value::Integer(9)]]);
    assert_eq!(
        fixture
            .page("SELECT id FROM samples LIMIT 500", 0, None)
            .len(),
        500
    );
    assert!(!fixture.db.result.as_ref().unwrap().truncated);
}

#[test]
fn pages_cannot_bypass_parser_authorizer_or_invalid_column_checks() {
    let mut fixture = Fixture::new();
    for sql in [
        "SELECT 1; SELECT 2",
        "DELETE FROM samples RETURNING id",
        "PRAGMA query_only=0",
        "SELECT load_extension('forbidden')",
        "SELECT absent FROM samples",
    ] {
        fixture
            .db
            .dispatch(DbCommand::ExecutePage(QueryPageRequest {
                sql: sql.into(),
                offset: 0,
                sort: None,
            }))
            .unwrap();
        drain(&mut fixture.db);
        assert!(fixture.db.last_error.is_some(), "accepted {sql}");
        assert!(fixture.db.result.is_none());
    }
    fixture
        .db
        .dispatch(DbCommand::ExecutePage(QueryPageRequest {
            sql: "SELECT id FROM samples".into(),
            offset: 0,
            sort: Some(ColumnSort {
                column: 1,
                descending: false,
            }),
        }))
        .unwrap();
    drain(&mut fixture.db);
    assert!(fixture.db.last_error.as_ref().unwrap().contains("Coluna"));
    assert_eq!(
        fixture.page("SELECT count(*) FROM samples", 0, None)[0][0],
        Value::Integer(1205)
    );
}

#[test]
fn cancellation_discards_pending_page_and_reuses_existing_connection() {
    let mut fixture = Fixture::new();
    let before = fixture.page("SELECT * FROM pragma_data_version", 0, None)[0][0].clone();
    let writer = Connection::open(&fixture.path).unwrap();
    writer
        .execute("UPDATE samples SET label='changed' WHERE id=1", [])
        .unwrap();
    let after = fixture.page("SELECT * FROM pragma_data_version", 0, None)[0][0].clone();
    assert_ne!(
        before, after,
        "pages must reuse the existing SQLite connection"
    );
    fixture.db.dispatch(DbCommand::ExecutePage(QueryPageRequest {
        sql: "WITH RECURSIVE endless(x) AS (SELECT 0 UNION ALL SELECT x+1 FROM endless) SELECT sum(x) FROM endless".into(),
        offset: 0, sort: None,
    })).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !fixture.db.is_running() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    thread::sleep(Duration::from_millis(20));
    fixture.db.dispatch(DbCommand::Cancel).unwrap();
    drain(&mut fixture.db);
    assert!(fixture.db.result.is_none());
    assert!(fixture.db.last_error.is_some());
    assert!(fixture.db.connected);
    assert_eq!(
        fixture.page("SELECT label FROM samples WHERE id=1", 0, None)[0][0],
        Value::Text("changed".into())
    );
}

#[test]
fn byte_limited_pages_continue_at_the_number_of_retained_rows() {
    let mut fixture = Fixture::new();
    let sql =
        "SELECT id, replace(hex(zeroblob(30000)), '0', 'z') FROM samples WHERE id<=100 ORDER BY id";
    let first = fixture.page(sql, 0, None);
    assert!(first.len() < QUERY_LIMIT && !first.is_empty());
    assert!(fixture.db.result.as_ref().unwrap().truncated);
    let next = fixture.page(sql, first.len() as u64, None);
    assert_eq!(next[0][0], Value::Integer(first.len() as i64 + 1));
    assert_eq!(first.len() + next.len(), 100);
}
