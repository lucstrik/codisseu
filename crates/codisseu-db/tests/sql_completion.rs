use codisseu_db::{
    remote::{DbObject, RemoteColumn},
    sql_completion::{self, Kind},
    sql_text::SqlDialect,
};
use std::collections::BTreeMap;

fn context(marked: &str) -> Option<sql_completion::Context> {
    let byte = marked.find('|').unwrap();
    let cursor = marked[..byte].chars().count();
    sql_completion::context(&marked.replacen('|', "", 1), cursor, SqlDialect::Mysql)
}
fn column(name: &str) -> RemoteColumn {
    RemoteColumn {
        name: name.into(),
        data_type: "text".into(),
        primary_key: false,
        nullable: true,
    }
}
type Catalog = (
    Vec<String>,
    BTreeMap<String, Vec<DbObject>>,
    BTreeMap<(String, String), Vec<RemoteColumn>>,
);
fn catalog() -> Catalog {
    let namespaces = vec!["corrier".into(), "corrier_fat".into()];
    let objects = BTreeMap::from([(
        "corrier".into(),
        vec![DbObject {
            namespace: "corrier".into(),
            name: "remetentes".into(),
            kind: "table".into(),
            columns: vec![],
        }],
    )]);
    let columns = BTreeMap::from([
        (
            ("corrier".into(), "remetentes".into()),
            vec![column("name"), column("id"), column("ação")],
        ),
        (
            ("corrier_fat".into(), "remetentes".into()),
            vec![column("invoice")],
        ),
    ]);
    (namespaces, objects, columns)
}
fn suggest(marked: &str) -> Vec<sql_completion::Suggestion> {
    let ctx = context(marked).expect("completion context");
    let (namespaces, objects, columns) = catalog();
    sql_completion::suggestions(
        &ctx,
        SqlDialect::Mysql,
        "corrier",
        &namespaces,
        &objects,
        &columns,
    )
}

#[test]
fn namespace_tables_alias_columns_and_fully_qualified_columns() {
    assert_eq!(
        suggest("SELECT `odd alias`.na| FROM corrier.remetentes AS `odd alias`;")[0].label,
        "name"
    );
    assert_eq!(
        suggest("SELECT `select`.na| FROM corrier.remetentes AS `select`;")[0].label,
        "name"
    );
    assert!(
        suggest("SELECT * FROM cor|;")
            .iter()
            .any(|s| s.label == "corrier" && s.kind == Kind::Namespace)
    );
    assert!(
        suggest("SELECT * FROM corrier.rem|;")
            .iter()
            .any(|s| s.label == "remetentes" && s.kind == Kind::Table)
    );
    assert_eq!(
        suggest("SELECT r.na| FROM corrier.remetentes AS r;")
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>(),
        vec!["name"]
    );
    assert_eq!(
        suggest("SELECT corrier.remetentes.na| FROM corrier_fat.remetentes r;")
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>(),
        vec!["name"]
    );
    assert_eq!(
        suggest("SELECT r.inv| FROM corrier_fat.remetentes r;")
            .iter()
            .map(|s| s.label.as_str())
            .collect::<Vec<_>>(),
        vec!["invoice"]
    );
}

#[test]
fn keywords_are_sql_tokens_and_current_statement_relations_are_isolated() {
    assert_eq!(
        suggest("SEL|;")
            .iter()
            .find(|s| s.kind == Kind::Keyword)
            .unwrap()
            .insert,
        "SELECT"
    );
    assert_eq!(
        suggest("SELECT * FROM corrier.remetentes r;\nSELECT r.na|;")
            .iter()
            .filter(|s| s.kind == Kind::Column)
            .count(),
        0
    );
    assert!(
        context("SELECT 1;\n|\nSELECT 2;").is_none(),
        "whitespace gap must not panic or suggest another statement"
    );
    assert!(
        context("SELECT |`name` FROM corrier.remetentes;").is_none(),
        "opening quote boundary must not panic"
    );
}

#[test]
fn unicode_ranges_and_quoted_identifier_prefixes() {
    let marked = "-- café 🦀\nSELECT r.aç| FROM corrier.remetentes r;";
    let ctx = context(marked).unwrap();
    assert_eq!(ctx.prefix, "aç");
    let text = marked.replacen('|', "", 1);
    assert_eq!(
        text.chars()
            .skip(ctx.range.start)
            .take(ctx.range.len())
            .collect::<String>(),
        "aç"
    );
    assert_eq!(suggest(marked)[0].label, "ação");
    assert_eq!(
        context("SELECT r.`na`| FROM corrier.remetentes r;")
            .unwrap()
            .prefix,
        "na"
    );
    assert_eq!(
        suggest("SELECT r.`na`| FROM corrier.remetentes r;")[0].insert,
        "`name`"
    );
    assert_eq!(
        context("SELECT r.`a``b|` FROM corrier.remetentes r;")
            .unwrap()
            .prefix,
        "a`b"
    );
}

#[test]
fn strings_comments_and_dollar_quotes_have_no_suggestions() {
    for text in [
        "SELECT 'na|';",
        "SELECT \"na|\";",
        "SELECT 1 -- na|\n;",
        "SELECT /* na| */ 1;",
        "SELECT # na|\n 1;",
    ] {
        assert!(context(text).is_none(), "{text}");
    }
    for marked in [
        "SELECT $$na|$$;",
        "SELECT $tag$na|$tag$;",
        "SELECT $á$na|$á$;",
    ] {
        let byte = marked.find('|').unwrap();
        let cursor = marked[..byte].chars().count();
        assert!(
            sql_completion::context(&marked.replacen('|', "", 1), cursor, SqlDialect::Postgres)
                .is_none()
        );
    }
}

#[test]
fn every_scalar_cursor_is_safe_and_returned_ranges_belong_to_the_document() {
    let documents = [
        "SELECT 1;\n\n SELECT r.`a``b`, '🦀;x' FROM corrier.remetentes r;  ",
        "-- ação 🦀\r\nSELECT \"a\"\"b\", $$x;y$$ FROM \"corrier\".\"remetentes\" r;",
        "SELECT 'unfinished",
        "SELECT /* unfinished",
        "   ; ;  ",
    ];
    for text in documents {
        for dialect in [SqlDialect::Mysql, SqlDialect::Postgres, SqlDialect::Sqlite] {
            for cursor in 0..=text.chars().count() + 1 {
                if let Some(ctx) = sql_completion::context(text, cursor, dialect) {
                    assert!(ctx.range.start <= ctx.range.end, "{text:?} @ {cursor}");
                    assert!(ctx.range.end <= text.chars().count(), "{text:?} @ {cursor}");
                }
            }
        }
    }
    let marked = "SELECT r.\"a\"\"b|\" FROM \"corrier\".\"remetentes\" r;";
    let byte = marked.find('|').unwrap();
    let ctx = sql_completion::context(
        &marked.replacen('|', "", 1),
        marked[..byte].chars().count(),
        SqlDialect::Postgres,
    )
    .unwrap();
    assert_eq!(ctx.prefix, "a\"b");
}

#[test]
fn hostile_identifier_is_quoted_and_escaped_as_one_identifier() {
    let ctx = context("SELECT * FROM corrier.|;").unwrap();
    let name = "a`; DROP TABLE users; --";
    let objects = BTreeMap::from([(
        "corrier".into(),
        vec![DbObject {
            namespace: "corrier".into(),
            name: name.into(),
            kind: "table".into(),
            columns: vec![],
        }],
    )]);
    let suggestions =
        sql_completion::suggestions(&ctx, SqlDialect::Mysql, "", &[], &objects, &BTreeMap::new());
    assert_eq!(suggestions[0].insert, "`a``; DROP TABLE users; --`");
    assert_eq!(
        sql_completion::quote_identifier("a\"; drop table x;--", SqlDialect::Postgres),
        "\"a\"\"; drop table x;--\""
    );
    for (dialect, name, expected) in [
        (SqlDialect::Postgres, "MixedCase", "\"MixedCase\""),
        (SqlDialect::Postgres, "table", "\"table\""),
        (SqlDialect::Mysql, "ordinary", "`ordinary`"),
    ] {
        assert_eq!(sql_completion::quote_identifier(name, dialect), expected);
    }
}

#[test]
fn joined_duplicate_columns_offer_qualified_aliases_instead_of_ambiguous_bare_names() {
    let ctx = context(
        "SELECT i| FROM corrier.remetentes AS r JOIN corrier_fat.fatcte AS f ON r.id=f.id;",
    )
    .unwrap();
    let columns = BTreeMap::from([
        (
            ("corrier".into(), "remetentes".into()),
            vec![column("id"), column("name")],
        ),
        (
            ("corrier_fat".into(), "fatcte".into()),
            vec![column("id"), column("invoice")],
        ),
    ]);
    let items =
        sql_completion::suggestions(&ctx, SqlDialect::Mysql, "", &[], &BTreeMap::new(), &columns);
    let ids: Vec<_> = items
        .iter()
        .filter(|item| item.kind == Kind::Column && item.label.ends_with(".id"))
        .collect();
    assert_eq!(
        ids.iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["f.id", "r.id"]
    );
    assert_eq!(
        ids.iter()
            .map(|item| item.insert.as_str())
            .collect::<Vec<_>>(),
        vec!["`f`.`id`", "`r`.`id`"]
    );
    assert!(!items.iter().any(|item| item.label == "id"));
    assert_eq!(
        items
            .iter()
            .find(|item| item.label == "invoice")
            .unwrap()
            .insert,
        "`invoice`"
    );

    let explicit = context(
        "SELECT r.i| FROM corrier.remetentes AS r JOIN corrier_fat.fatcte AS f ON r.id=f.id;",
    )
    .unwrap();
    let items = sql_completion::suggestions(
        &explicit,
        SqlDialect::Mysql,
        "",
        &[],
        &BTreeMap::new(),
        &columns,
    );
    assert_eq!(
        items.iter().find(|item| item.label == "id").unwrap().insert,
        "`id`"
    );

    let quoted = context(
        "SELECT i| FROM corrier.remetentes AS `r``odd` JOIN corrier_fat.fatcte AS f ON 1=1;",
    )
    .unwrap();
    let items = sql_completion::suggestions(
        &quoted,
        SqlDialect::Mysql,
        "",
        &[],
        &BTreeMap::new(),
        &columns,
    );
    let insert = &items
        .iter()
        .find(|item| item.label == "r`odd.id")
        .unwrap()
        .insert;
    assert_eq!(insert, "`r``odd`.`id`");
    let sql = format!(
        "SELECT {insert} FROM corrier.remetentes AS `r``odd` JOIN corrier_fat.fatcte AS f ON 1=1;"
    );
    let parsed =
        sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::MySqlDialect {}, &sql).unwrap();
    let sqlparser::ast::Statement::Query(query) = &parsed[0] else {
        panic!("SELECT expected")
    };
    let sqlparser::ast::SetExpr::Select(select) = query.body.as_ref() else {
        panic!("SELECT expected")
    };
    let sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::CompoundIdentifier(
        identifiers,
    )) = &select.projection[0]
    else {
        panic!("qualified column expected")
    };
    assert_eq!(
        identifiers
            .iter()
            .map(|id| id.value.as_str())
            .collect::<Vec<_>>(),
        vec!["r`odd", "id"]
    );

    // The first completed metadata response must not publish a temporarily
    // ambiguous bare id while the other relation is still loading.
    let partial = BTreeMap::from([(("corrier".into(), "remetentes".into()), vec![column("id")])]);
    let items =
        sql_completion::suggestions(&ctx, SqlDialect::Mysql, "", &[], &BTreeMap::new(), &partial);
    assert_eq!(
        items
            .iter()
            .find(|item| item.kind == Kind::Column)
            .unwrap()
            .insert,
        "`r`.`id`"
    );
}
