use codisseu_db::sql_text::{SqlDialect::*, execution_range, statement_range};

fn at(text: &str, marker: &str) -> usize {
    text[..text.find(marker).unwrap()].chars().count()
}
fn target(text: &str, marker: &str, dialect: codisseu_db::sql_text::SqlDialect) -> String {
    let range = statement_range(text, at(text, marker), dialect).unwrap();
    text.chars().skip(range.start).take(range.len()).collect()
}

#[test]
fn current_statement_multiline_boundaries_and_unicode_positions() {
    let text = "-- Olá 🦀\nSELECT 'ação; café';  \n\nSELECT\n  22 AS second;\n\n";
    assert_eq!(
        target(text, "ação", Postgres),
        "-- Olá 🦀\nSELECT 'ação; café';"
    );
    assert_eq!(target(text, "22", Postgres), "SELECT\n  22 AS second;");
    assert_eq!(
        target(text, ";  ", Postgres),
        "-- Olá 🦀\nSELECT 'ação; café';"
    );
    let semi = at(text, ";  ");
    assert_eq!(statement_range(text, semi + 2, Postgres).unwrap().start, 0);
    let newline = at(text, "\n\nSELECT") + 1;
    let next = statement_range(text, newline, Postgres).unwrap();
    assert_eq!(
        text.chars()
            .skip(next.start)
            .take(next.len())
            .collect::<String>(),
        "SELECT\n  22 AS second;"
    );
    let eof = statement_range(text, text.chars().count(), Postgres).unwrap();
    assert_eq!(eof, next);
    assert!(statement_range(text, text.chars().count() + 1, Postgres).is_err());
}

#[test]
fn quoted_strings_identifiers_comments_and_dollar_bodies_do_not_split() {
    let cases = [
        (
            Sqlite,
            "SELECT 'it''s;still', \"a;\"\"b\", `c;``d`, [e;f];\nSELECT 2;",
        ),
        (
            Postgres,
            "/* one; /* nested; */ still; */ SELECT $$a;b$$, $tag$a;b$tag$, $áé$a;b$áé$, E'it\\';s';\nSELECT 2;",
        ),
        (
            Mysql,
            "# header;\nSELECT 'a;''b', `table;name`, \"a;\"; -- note;\nSELECT 2;",
        ),
        (Postgres, "SELECT 1 -- comment;\n+ 1;\nSELECT 2;"),
        (Postgres, "SELECT identifier$tag$foo;\nSELECT 2;"),
    ];
    for (dialect, text) in cases {
        assert!(target(text, "SELECT 2", dialect).ends_with("SELECT 2;"));
        let first = target(text, "SELECT", dialect);
        assert!(!first.contains("SELECT 2"), "{first}");
    }
    assert_eq!(
        target("SELECT 1--2; SELECT 3;", "SELECT 3", Mysql),
        "SELECT 3;"
    );
    assert_eq!(
        target("SELECT 1--2; SELECT 3;", "SELECT 3", Postgres),
        "SELECT 1--2; SELECT 3;"
    );
}

#[test]
fn explicit_selection_is_exact_and_backend_validates_scripts() {
    let text = "SELECT '🦀';\nSELECT 22;\nSELECT 33;";
    let start = at(text, "22");
    assert_eq!(
        execution_range(text, 0, Some(start..start + 2), Postgres).unwrap(),
        start..start + 2
    );
    let all = 0..text.chars().count();
    assert_eq!(
        execution_range(text, 0, Some(all.clone()), Postgres).unwrap(),
        all
    );
    let reversed = RangeForTest::reversed();
    assert!(execution_range(text, 0, Some(reversed), Postgres).is_err());
    assert!(execution_range(text, 0, Some(0..500), Postgres).is_err());
}
// Construct a reversed range without triggering the compiler's intentional
// reversed-empty-range lint; this is an invalid external input regression.
struct RangeForTest;
impl RangeForTest {
    fn reversed() -> std::ops::Range<usize> {
        let start = 5;
        let end = 2;
        start..end
    }
}

#[test]
fn malformed_or_mode_dependent_current_statement_is_rejected_without_guessing() {
    for (dialect, tail) in [
        (Postgres, "SELECT 'unfinished; SELECT 2;"),
        (Postgres, "SELECT $tag$unfinished; SELECT 2;"),
        (Postgres, "SELECT 2 /* unfinished;"),
        (Mysql, "SELECT 'a\\'; SELECT 2;"),
        (Postgres, "SELECT 'a\\'; SELECT 2;"),
        (Mysql, "/*! SELECT 2; */ SELECT 3;"),
    ] {
        let text = format!("SELECT 1;\n{tail}");
        assert_eq!(target(&text, "SELECT 1", dialect), "SELECT 1;");
        assert!(
            statement_range(&text, text.chars().count(), dialect).is_err(),
            "{text}"
        );
        // Explicitly selected SQL bypasses boundary guessing, not backend guards.
        assert!(execution_range(&text, 0, Some(0..text.chars().count()), dialect).is_ok());
    }
    for text in ["", " \n\t", "-- only a comment;", "/* only; */", ";;;"] {
        assert!(statement_range(text, 0, Postgres).is_err(), "{text}");
    }
}
