//! Format contracts tested independently of the UI and reference renderers.
use codisseu_db::{
    remote::{Cell, TableRef},
    transfer::{self, SqlFlavor, TransferData, TransferFormat, TransferOptions},
    transfer_formats_extra,
};
use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

fn fixture() -> TransferData {
    TransferData {
        columns: [
            "id",
            "label",
            "nullable",
            "exact",
            "formula",
            "payload",
            "flag",
            "json_data",
        ]
        .map(str::to_owned)
        .to_vec(),
        rows: vec![
            vec![
                Cell::Number("1".into()),
                Cell::Text("Ação 🦀 \"quoted\" O'Reilly, pipe| ; <script>&\nlinha\tcol".into()),
                Cell::Null,
                Cell::Number("123456789012345678901.123456789".into()),
                Cell::Text("=SUM(A1:A2)".into()),
                Cell::Binary(vec![0, 255, 13, 10]),
                Cell::Bool(true),
                Cell::Json(r#"{"nested":true,"text":"<>&"}"#.into()),
            ],
            vec![
                Cell::Number("2".into()),
                Cell::Text("Grace\nHopper".into()),
                Cell::Text(String::new()),
                Cell::Number("-0.00".into()),
                Cell::Text("+cmd|bad".into()),
                Cell::Binary(vec![]),
                Cell::Bool(false),
                Cell::Json("[1,true,null]".into()),
            ],
        ],
        source_truncated: false,
    }
}
fn options(format: TransferFormat) -> TransferOptions {
    TransferOptions {
        format,
        sql_flavor: SqlFlavor::Postgres,
        sql_table: Some(TableRef {
            namespace: "public".into(),
            name: "fixture_table".into(),
        }),
        query: Some("SELECT * FROM public.fixture_table;\n-- synthetic transfer fixture".into()),
        ..Default::default()
    }
}
fn formats() -> Vec<(&'static str, &'static str, TransferFormat, &'static str)> {
    let extra = |id: &str| TransferFormat::Extra(id.to_owned());
    vec![
        (
            "sql-inserts",
            "SQL Inserts",
            TransferFormat::SqlInsert,
            "sql",
        ),
        ("sql-updates", "SQL Updates", extra("sql-updates"), "sql"),
        ("where-clause", "Where Clause", extra("where-clause"), "sql"),
        ("xlsx", "Excel (xlsx)", extra("xlsx"), "xlsx"),
        ("csv", "CSV", TransferFormat::Csv, "csv"),
        ("tsv", "TSV", TransferFormat::Tsv, "tsv"),
        ("pipe", "Pipe-separated", extra("pipe"), "psv"),
        (
            "semicolon",
            "Semicolon-separated",
            extra("semicolon"),
            "csv",
        ),
        ("csv-scripted", "CSV (script)", extra("csv-scripted"), "csv"),
        ("html-groovy", "HTML (groovy)", extra("html-groovy"), "html"),
        ("html-js", "HTML (js)", extra("html-js"), "html"),
        ("json", "JSON", TransferFormat::Json, "json"),
        ("markdown", "Markdown", extra("markdown"), "md"),
        ("one-row", "One-row", extra("one-row"), "txt"),
        ("pretty", "Pretty", extra("pretty"), "txt"),
        (
            "python-dataframe",
            "Python-DataFrame",
            extra("python-dataframe"),
            "py",
        ),
        (
            "sql-insert-multirow",
            "SQL-Insert-Multirow",
            extra("sql-insert-multirow"),
            "sql",
        ),
        (
            "sql-insert-statements",
            "SQL-Insert-Statements",
            extra("sql-insert-statements"),
            "sql",
        ),
        ("xml", "XML", extra("xml"), "xml"),
    ]
}

#[test]
fn all_nineteen_extractors_generate_real_outputs_and_optional_independent_corpus() {
    let formats = formats();
    assert_eq!(formats.len(), 19);
    let ids = formats
        .iter()
        .map(|(id, ..)| *id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 19);
    assert_eq!(transfer_formats_extra::EXTRACTORS.len(), 15);
    for (id, _) in transfer_formats_extra::EXTRACTORS {
        assert!(ids.contains(id), "Missing {id}");
    }
    let data = fixture();
    let directory = std::env::var_os("CODISSEU_TRANSFER_EXPORT_DIR").map(std::path::PathBuf::from);
    if let Some(directory) = &directory {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(
            directory.join("fixture.json"),
            serde_json::to_vec_pretty(&data).unwrap(),
        )
        .unwrap();
    }
    let mut manifest = Vec::new();
    for (index, (id, label, format, extension)) in formats.into_iter().enumerate() {
        let output =
            transfer::encode(&options(format), &data).unwrap_or_else(|e| panic!("{label}: {e}"));
        assert!(!output.is_empty(), "{label}");
        if id == "xlsx" {
            assert_eq!(&output[..4], b"PK\x03\x04");
            assert!(output.contains(&0));
            assert!(output.windows(4).any(|w| w == b"PK\x05\x06"));
        } else {
            let text = std::str::from_utf8(&output).unwrap();
            assert!(text.contains("Ação"), "{label} lost Unicode data");
            assert!(
                text.contains("123456789012345678901.123456789"),
                "{label} lost decimal precision"
            );
            assert!(
                text.contains("=SUM(A1:A2)"),
                "{label} lost formula-like literal text"
            );
        }
        let filename = format!("{index:02}-{id}.{extension}");
        if let Some(directory) = &directory {
            std::fs::write(directory.join(&filename), &output).unwrap();
        }
        manifest
            .push(serde_json::json!({"id":id,"label":label,"file":filename,"bytes":output.len()}));
    }
    if let Some(directory) = directory {
        std::fs::write(directory.join("manifest.json"),serde_json::to_vec_pretty(&serde_json::json!({"formats":manifest,"query":options(TransferFormat::Csv).query.unwrap(),"scope":"synthetic typed fixture; no database reads or writes"})).unwrap()).unwrap();
    }
}

#[test]
fn update_where_uses_proven_keys_while_full_where_clause_can_compare_every_field() {
    let data = fixture();
    let mut update = options(TransferFormat::Extra("sql-updates".into()));
    update.sql_key_columns = vec!["id".into()];
    let sql = String::from_utf8(transfer::encode(&update, &data).unwrap()).unwrap();
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, &sql).unwrap();
    assert_eq!(statements.len(), 2);
    for statement in statements {
        let Statement::Update {
            selection: Some(selection),
            assignments,
            ..
        } = statement
        else {
            panic!("UPDATE must have WHERE")
        };
        assert_eq!(assignments.len(), data.columns.len());
        let condition = selection.to_string();
        assert!(condition.starts_with("\"id\" = "));
        assert!(!condition.contains("label"));
        assert!(!condition.contains("formula"));
    }
    let mut where_options = options(TransferFormat::Extra("where-clause".into()));
    where_options.sql_key_columns = vec!["id".into()];
    let all = String::from_utf8(transfer::encode(&where_options, &data).unwrap()).unwrap();
    for column in &data.columns {
        assert!(all.contains(&format!("\"{column}\"")), "missing {column}");
    }
    assert!(all.contains("\"nullable\" IS NULL"));
    assert!(all.contains("\"nullable\" = ''") || all.contains("\"nullable\" = E''"));
    assert!(all.contains("\nOR "));
    // Removing known keys still produces a restricting predicate, never a broad UPDATE.
    update.sql_key_columns.clear();
    let sql = String::from_utf8(transfer::encode(&update, &data).unwrap()).unwrap();
    let statements = Parser::parse_sql(&PostgreSqlDialect {}, &sql).unwrap();
    for statement in statements {
        let Statement::Update {
            selection: Some(condition),
            ..
        } = statement
        else {
            panic!("Unrestricted UPDATE")
        };
        let condition = condition.to_string();
        assert!(condition.contains("\"id\""));
        assert!(condition.contains("\"nullable\""));
    }
}

#[test]
fn xml_html_reject_controls_while_xlsx_uses_ooxml_xstring_encoding() {
    let data = TransferData {
        columns: vec!["value".into()],
        rows: vec![vec![Cell::Text("control\u{1}".into())]],
        source_truncated: false,
    };
    for id in ["xml", "html-groovy", "html-js"] {
        assert!(
            transfer::encode(&options(TransferFormat::Extra(id.into())), &data).is_err(),
            "{id}"
        );
    }
    let xlsx = transfer::encode(&options(TransferFormat::Extra("xlsx".into())), &data).unwrap();
    assert!(
        xlsx.windows(b"control_x0001_".len())
            .any(|v| v == b"control_x0001_")
    );
    assert!(
        !xlsx
            .windows(b"control\x01".len())
            .any(|v| v == b"control\x01")
    );
    assert_eq!(
        codisseu_db::transfer_xlsx::decode(&xlsx, true).unwrap(),
        data
    );
    assert!(transfer::encode(&options(TransferFormat::Json), &data).is_ok());
}

#[test]
fn html_escapes_markup_markdown_escapes_pipes_and_xlsx_keeps_big_numbers_as_text() {
    let data = fixture();
    for id in ["html-groovy", "html-js"] {
        let html = String::from_utf8(
            transfer::encode(&options(TransferFormat::Extra(id.into())), &data).unwrap(),
        )
        .unwrap();
        assert!(html.contains("&lt;script&gt;&amp;"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("<br/>"));
    }
    let markdown = String::from_utf8(
        transfer::encode(&options(TransferFormat::Extra("markdown".into())), &data).unwrap(),
    )
    .unwrap();
    assert!(markdown.contains("pipe&#124;"));
    assert!(markdown.contains("&lt;script&gt;"));
    let xlsx = transfer::encode(&options(TransferFormat::Extra("xlsx".into())), &data).unwrap();
    for contract in [
        r#"r="D2" t="inlineStr""#,
        r#"r="E2" t="inlineStr""#,
        r#"name="Result 1""#,
        r#"name="Query""#,
    ] {
        assert!(
            xlsx.windows(contract.len())
                .any(|bytes| bytes == contract.as_bytes()),
            "Missing XLSX {contract}"
        );
    }
    assert!(!xlsx.windows(3).any(|bytes| bytes == b"<f>"));
}
