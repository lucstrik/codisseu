use codisseu_db::{
    remote::Cell,
    transfer::{self, TransferData, TransferFormat, TransferOptions},
    transfer_xlsx,
};
use std::io::{Cursor, Write};
fn archive(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        zip.start_file(
            *name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
fn fixture(sheet: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut parts = vec![
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0"?><workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Query" r:id="q"/><sheet name="Result 1" r:id="data"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships><Relationship Id="q" Target="worksheets/query.xml"/><Relationship Id="data" Target="worksheets/data.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/query.xml",
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>SELECT * FROM fixture;</t></is></c></row></sheetData></worksheet>"#,
        ),
        ("xl/worksheets/data.xml", sheet),
    ];
    parts.extend_from_slice(extra);
    archive(&parts)
}
#[test]
fn xlsx_shared_inline_exact_numbers_nulls_bools_dates_and_preferred_result_sheet() {
    let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>number</t></is></c><c r="B1" t="inlineStr"><is><t>text</t></is></c><c r="C1" t="inlineStr"><is><t>missing</t></is></c><c r="D1" t="inlineStr"><is><t>empty</t></is></c><c r="E1" t="inlineStr"><is><t>bool</t></is></c><c r="F1" t="inlineStr"><is><t>date</t></is></c></row><row r="2"><c r="A2"><v>900719925474099312345.00100</v></c><c r="B2" t="s"><v>0</v></c><c r="D2" t="inlineStr"><is><t></t></is></c><c r="E2" t="b"><v>1</v></c><c r="F2" s="1"><v>45210</v></c></row></sheetData></worksheet>"#;
    let bytes = fixture(
        sheet,
        &[
            (
                "xl/sharedStrings.xml",
                r#"<sst><si><r><t>=1&amp;</t></r><r><t>2&#10;😀</t></r></si></sst>"#,
            ),
            (
                "xl/styles.xml",
                r#"<styleSheet><cellXfs><xf numFmtId="0"/><xf numFmtId="14"/></cellXfs></styleSheet>"#,
            ),
        ],
    );
    let result = transfer_xlsx::decode(&bytes, true).unwrap();
    assert_eq!(
        result.columns,
        vec!["number", "text", "missing", "empty", "bool", "date"]
    );
    assert_eq!(
        result.rows,
        vec![vec![
            Cell::Number("900719925474099312345.00100".into()),
            Cell::Text("=1&2\n😀".into()),
            Cell::Null,
            Cell::Text("".into()),
            Cell::Bool(true),
            Cell::Text("45210".into())
        ]]
    );
}
#[test]
fn xlsx_export_import_roundtrip_without_float_or_formula_evaluation() {
    let data = TransferData {
        columns: vec!["id".into(), "text".into(), "nil".into(), "boolean".into()],
        rows: vec![vec![
            Cell::Number("900719925474099312345".into()),
            Cell::Text("=SUM(A1) & <safe> 😀".into()),
            Cell::Null,
            Cell::Bool(false),
        ]],
        source_truncated: false,
    };
    let options = TransferOptions {
        format: TransferFormat::Extra("xlsx".into()),
        query: Some("SELECT exact FROM fixture".into()),
        ..Default::default()
    };
    let bytes = transfer::encode(&options, &data).unwrap();
    let decoded = transfer_xlsx::decode(&bytes, true).unwrap();
    assert_eq!(
        decoded.rows[0][0],
        Cell::Text("900719925474099312345".into()),
        "exact long numbers are intentionally text in Excel"
    );
    assert_eq!(decoded.rows[0][1..], data.rows[0][1..]);
    assert_eq!(decoded.columns, data.columns);
}
#[test]
fn xlsx_rejects_formulas_merges_external_entities_links_and_budget_overflow() {
    for contents in [r#"<f>1+2</f><v>3</v>"#, r#"<is><t>&custom;</t></is>"#] {
        let sheet = format!(
            "<worksheet><sheetData><row r=\"1\"><c r=\"A1\">{contents}</c></row></sheetData></worksheet>"
        );
        assert!(transfer_xlsx::decode(&fixture(&sheet, &[]), false).is_err());
    }
    for sheet in [
        "<!DOCTYPE worksheet [<!ENTITY custom SYSTEM 'file:///etc/passwd'>]><worksheet/>",
        "<worksheet><mergeCells><mergeCell ref='A1:B1'/></mergeCells></worksheet>",
        "<worksheet><sheetData><row r='502'><c r='A502'><v>1</v></c></row></sheetData></worksheet>",
        "<worksheet><sheetData><row r='1'><c r='IW1'><v>1</v></c></row></sheetData></worksheet>",
    ] {
        assert!(transfer_xlsx::decode(&fixture(sheet, &[]), false).is_err());
    }
    let bytes = fixture(
        "<worksheet/>",
        &[(
            "xl/_rels/external.xml.rels",
            "<Relationships><Relationship Id='x' TargetMode='External' Target='https://example.invalid/a'/></Relationships>",
        )],
    );
    assert!(transfer_xlsx::decode(&bytes, false).is_err());
    let expanded = "x".repeat(transfer::MAX_TRANSFER_FILE + 1);
    assert!(transfer_xlsx::decode(&archive(&[("bomb.xml", &expanded)]), false).is_err());
}
#[test]
fn delimited_extra_formats_import_with_real_separator() {
    for (id, text) in [
        ("pipe", "id|value\n1|hello\n"),
        ("semicolon", "id;value\n1;hello\n"),
        ("csv-scripted", "id,value\n1,hello\n"),
    ] {
        let options = TransferOptions {
            format: TransferFormat::Extra(id.into()),
            ..Default::default()
        };
        let data = transfer::decode(&options, text).unwrap();
        assert_eq!(data.columns, vec!["id", "value"]);
        assert_eq!(data.rows[0][1], Cell::Text("hello".into()));
    }
}

#[test]
fn xlsx_string_escaping_preserves_literal_patterns_cr_controls_and_unicode() {
    let text = "literal _x0041_ and _x000D_ and _x005F_\r\n\t\0😀";
    let data = TransferData {
        columns: vec!["value".into()],
        rows: vec![vec![Cell::Text(text.into())]],
        source_truncated: false,
    };
    let options = TransferOptions {
        format: TransferFormat::Extra("xlsx".into()),
        ..Default::default()
    };
    let bytes = transfer::encode(&options, &data).unwrap();
    let decoded = transfer_xlsx::decode(&bytes, true).unwrap();
    assert_eq!(decoded, data);
    let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>_x005F_x0041_ _x000D_ _xD83D__xDE00_</t></is></c></row></sheetData></worksheet>"#;
    let result = transfer_xlsx::decode(&fixture(sheet, &[]), false).unwrap();
    assert_eq!(result.rows[0][0], Cell::Text("_x0041_ \r 😀".into()));
}
#[test]
#[ignore = "requires explicit local DataGrip reference export fixture; never embedded"]
fn actual_datagrip_export_imports_as_bounded_snapshot() {
    let path = std::env::var("CODISSEU_DATAGRIP_XLSX").expect("reference fixture path");
    let bytes = std::fs::read(path).unwrap();
    let data = transfer_xlsx::decode(&bytes, true).unwrap();
    assert_eq!(data.rows.len(), 3);
    assert!(!data.columns.is_empty());
    println!(
        "DataGrip reference XLSX: {} rows, {} columns",
        data.rows.len(),
        data.columns.len()
    );
}
