//! Independent equivalents of the extractors observed in DataGrip DB-262.10315.24.
//! Public format specifications and runtime evidence: docs/m4-data-transfer-contract.md.
use crate::{
    remote::Cell,
    transfer::{self, SqlFlavor, TransferData, TransferFormat, TransferOptions},
};
use std::fmt::Write as _;

pub const EXTRACTORS: &[(&str, &str)] = &[
    ("sql-updates", "SQL Updates"),
    ("where-clause", "Where Clause"),
    ("xlsx", "Excel (xlsx)"),
    ("pipe", "Pipe-separated"),
    ("semicolon", "Semicolon-separated"),
    ("csv-scripted", "CSV (script)"),
    ("html-groovy", "HTML (groovy)"),
    ("html-js", "HTML (js)"),
    ("markdown", "Markdown"),
    ("one-row", "One-row"),
    ("pretty", "Pretty"),
    ("python-dataframe", "Python-DataFrame"),
    ("sql-insert-multirow", "SQL-Insert-Multirow"),
    ("sql-insert-statements", "SQL-Insert-Statements"),
    ("xml", "XML"),
];

pub fn extension(id: &str) -> &'static str {
    match id {
        "xlsx" => "xlsx",
        "html-groovy" | "html-js" => "html",
        "xml" => "xml",
        "markdown" => "md",
        "python-dataframe" => "py",
        "pipe" => "psv",
        "semicolon" | "csv-scripted" => "csv",
        "sql-updates" | "where-clause" | "sql-insert-multirow" | "sql-insert-statements" => "sql",
        _ => "txt",
    }
}

pub fn encode(id: &str, options: &TransferOptions, data: &TransferData) -> Result<Vec<u8>, String> {
    match id {
        "xlsx" => xlsx(options, data),
        "pipe" | "semicolon" | "csv-scripted" => {
            let mut options = options.clone();
            options.format = TransferFormat::Csv;
            options.transpose = false;
            options.row_header = false;
            if id == "pipe" {
                options.delimiter = Some(b'|');
            }
            if id == "semicolon" {
                options.delimiter = Some(b';');
            }
            if id == "csv-scripted" {
                options.null_marker = "null".into();
            }
            transfer::encode(&options, data)
        }
        _ => {
            let text = match id {
                "html-groovy" | "html-js" => html(options, data)?,
                "xml" => xml(data)?,
                "markdown" => markdown(options, data),
                "one-row" => one_row(options, data)?,
                "pretty" => pretty(options, data),
                "python-dataframe" => python(data)?,
                "sql-updates" => updates(options, data)?,
                "where-clause" => conditions(options, data)?,
                "sql-insert-multirow" => multirow(options, data)?,
                "sql-insert-statements" => {
                    let mut opts = options.clone();
                    opts.format = TransferFormat::SqlInsert;
                    opts.transpose = false;
                    opts.row_header = false;
                    return transfer::encode(&opts, data);
                }
                _ => return Err(format!("Extrator desconhecido: {id}")),
            };
            if text.len() > transfer::MAX_TRANSFER_FILE {
                return Err("Exportação excede 8 MiB".into());
            }
            Ok(text.into_bytes())
        }
    }
}

fn text(cell: &Cell) -> String {
    if matches!(cell, Cell::Null) {
        "null".into()
    } else {
        cell.clipboard_text()
    }
}
fn xml_escape(value: &str) -> Result<String, String> {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => result.push_str("&amp;"), '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"), '"' => result.push_str("&quot;"), '\'' => result.push_str("&apos;"),
            '\t'|'\n'|'\r' => result.push(ch),
            ch if ch < '\u{20}' || matches!(ch,'\u{FFFE}'|'\u{FFFF}') => return Err("XML não aceita certos caracteres de controle; use JSON ou Codisseu JSON para preservar esses valores".into()),
            _ => result.push(ch),
        }
    }
    Ok(result)
}
fn html(options: &TransferOptions, data: &TransferData) -> Result<String, String> {
    let mut out = String::from(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"UTF-8\"><title>Result</title></head>\n<body>\n<table border=\"1\" style=\"border-collapse:collapse\">\n",
    );
    if options.header {
        out.push_str("<tr>\n");
        for name in &data.columns {
            writeln!(out, "  <th>{}</th>", xml_escape(name)?).unwrap();
        }
        out.push_str("</tr>\n");
    }
    for row in &data.rows {
        out.push_str("<tr>\n");
        for cell in row {
            writeln!(
                out,
                "  <td>{}</td>",
                xml_escape(&text(cell))?.replace('\n', "<br/>")
            )
            .unwrap();
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n</body>\n</html>\n");
    Ok(out)
}
fn xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c == '_' || c.is_alphabetic())
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}
fn xml(data: &TransferData) -> Result<String, String> {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<data>\n");
    for row in &data.rows {
        out.push_str("<row>\n");
        for (name, cell) in data.columns.iter().zip(row) {
            let value = xml_escape(&text(cell))?;
            if xml_name(name) {
                writeln!(out, "  <{name}>{value}</{name}>").unwrap();
            } else {
                writeln!(
                    out,
                    "  <column name=\"{}\">{value}</column>",
                    xml_escape(name)?
                )
                .unwrap();
            }
        }
        out.push_str("</row>\n");
    }
    out.push_str("</data>\n");
    Ok(out)
}
fn md(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
        .replace('\r', "")
        .replace('\n', "<br/>")
}
fn markdown(options: &TransferOptions, data: &TransferData) -> String {
    let mut out = String::new();
    let mut row = |values: Vec<String>| {
        out.push_str("| ");
        out.push_str(&values.join(" | "));
        out.push_str(" |\n");
    };
    if options.header {
        row(data.columns.iter().map(|v| md(v)).collect());
        row(data.columns.iter().map(|_| ":---".into()).collect());
    }
    for values in &data.rows {
        row(values.iter().map(|c| md(&text(c))).collect());
    }
    out
}
fn one_row(options: &TransferOptions, data: &TransferData) -> Result<String, String> {
    data.rows
        .iter()
        .flat_map(|r| r.iter())
        .map(|c| transfer::sql_literal(c, options.sql_flavor))
        .collect::<Result<Vec<_>, _>>()
        .map(|v| v.join(", "))
}
fn pretty(options: &TransferOptions, data: &TransferData) -> String {
    let mut rows: Vec<Vec<String>> = Vec::new();
    if options.header {
        rows.push(data.columns.clone());
    }
    rows.extend(
        data.rows
            .iter()
            .map(|row| row.iter().map(text).collect::<Vec<_>>()),
    );
    let widths: Vec<usize> = (0..data.columns.len())
        .map(|col| {
            rows.iter()
                .filter_map(|row| row.get(col))
                .flat_map(|v| v.lines())
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0)
                .min(256)
        })
        .collect();
    let border = format!(
        "+{}+\n",
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("+")
    );
    let mut out = border.clone();
    for (index, row) in rows.iter().enumerate() {
        let lines: Vec<Vec<String>> = row
            .iter()
            .enumerate()
            .map(|(col, v)| {
                v.split('\n')
                    .flat_map(|line| {
                        let chars: Vec<char> = line.chars().collect();
                        if chars.is_empty() {
                            vec![String::new()]
                        } else {
                            chars
                                .chunks(widths[col].max(1))
                                .map(|chunk| chunk.iter().collect())
                                .collect()
                        }
                    })
                    .collect()
            })
            .collect();
        for line in 0..lines.iter().map(Vec::len).max().unwrap_or(1) {
            out.push('|');
            for (col, width) in widths.iter().enumerate() {
                let content = lines[col].get(line).map(String::as_str).unwrap_or("");
                // Long values wrap instead of being silently truncated.
                let chars = content.chars().count();
                out.push_str(content);
                out.push_str(&" ".repeat(width.saturating_sub(chars)));
                out.push('|');
            }
            out.push('\n');
        }
        if options.header && index == 0 {
            out.push_str(&border);
        }
    }
    out.push_str(&border);
    out
}
fn python_cell(cell: &Cell) -> Result<String, String> {
    Ok(match cell {
        Cell::Null => "None".into(),
        Cell::Bool(v) => if *v { "True" } else { "False" }.into(),
        Cell::Number(v) => {
            transfer::sql_literal(cell, SqlFlavor::Postgres)?;
            format!("Decimal({})", serde_json::to_string(v).unwrap())
        }
        Cell::Binary(bytes) => format!("bytes.fromhex('{}')", transfer::hex(bytes)),
        Cell::Text(v) | Cell::Json(v) => serde_json::to_string(v).map_err(|e| e.to_string())?,
    })
}
fn python(data: &TransferData) -> Result<String, String> {
    if data
        .columns
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != data.columns.len()
    {
        return Err("Python DataFrame requer nomes de colunas únicos".into());
    }
    let mut out = String::from("import pandas as pd\nfrom decimal import Decimal\n\ndata = {\n");
    for (index, name) in data.columns.iter().enumerate() {
        let values = data
            .rows
            .iter()
            .map(|row| python_cell(&row[index]))
            .collect::<Result<Vec<_>, _>>()?;
        writeln!(
            out,
            "    {}: [{}],",
            serde_json::to_string(name).unwrap(),
            values.join(", ")
        )
        .unwrap();
    }
    out.push_str("}\ndf = pd.DataFrame(data)\n");
    Ok(out)
}
fn table(options: &TransferOptions) -> Result<String, String> {
    let table = options
        .sql_table
        .as_ref()
        .filter(|v| !v.name.trim().is_empty())
        .ok_or("Informe a tabela de destino do SQL exportado")?;
    Ok(transfer::qualified_table(table, options.sql_flavor))
}
fn row_condition(
    options: &TransferOptions,
    data: &TransferData,
    row: &[Cell],
) -> Result<String, String> {
    let mut indices = Vec::new();
    for key in &options.sql_key_columns {
        indices.push(
            data.columns
                .iter()
                .position(|n| n == key)
                .ok_or("Chave SQL não está no resultado")?,
        );
    }
    if indices.is_empty() {
        indices.extend(0..data.columns.len());
    }
    if indices.is_empty() {
        return Err("Não gerar condição SQL vazia".into());
    }
    indices
        .iter()
        .map(|&index| {
            let name = transfer::quote_identifier(&data.columns[index], options.sql_flavor);
            if matches!(row[index], Cell::Null) {
                Ok(format!("{name} IS NULL"))
            } else {
                Ok(format!(
                    "{name} = {}",
                    transfer::sql_literal(&row[index], options.sql_flavor)?
                ))
            }
        })
        .collect::<Result<Vec<_>, String>>()
        .map(|v| v.join(" AND "))
}
fn conditions(options: &TransferOptions, data: &TransferData) -> Result<String, String> {
    let mut options = options.clone();
    options.sql_key_columns.clear();
    data.rows
        .iter()
        .map(|row| row_condition(&options, data, row).map(|v| format!("({v})")))
        .collect::<Result<Vec<_>, _>>()
        .map(|v| v.join("\nOR "))
}
fn updates(options: &TransferOptions, data: &TransferData) -> Result<String, String> {
    let table = table(options)?;
    let mut out = String::new();
    for row in &data.rows {
        let assignments = data
            .columns
            .iter()
            .zip(row)
            .map(|(name, cell)| {
                Ok(format!(
                    "{} = {}",
                    transfer::quote_identifier(name, options.sql_flavor),
                    transfer::sql_literal(cell, options.sql_flavor)?
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        writeln!(
            out,
            "UPDATE {table} SET {} WHERE {};",
            assignments.join(", "),
            row_condition(options, data, row)?
        )
        .unwrap();
    }
    Ok(out)
}
fn multirow(options: &TransferOptions, data: &TransferData) -> Result<String, String> {
    let table = table(options)?;
    if data.rows.is_empty() {
        return Ok(String::new());
    }
    let names = data
        .columns
        .iter()
        .map(|v| transfer::quote_identifier(v, options.sql_flavor))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = data
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| transfer::sql_literal(c, options.sql_flavor))
                .collect::<Result<Vec<_>, _>>()
                .map(|v| format!("({})", v.join(", ")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(format!(
        "INSERT INTO {table} ({names})\nVALUES {};\n",
        rows.join(",\n       ")
    ))
}

// A small standards-based stored-ZIP writer: no runtime, external process, or renamed CSV.
// OOXML parts use inline strings; string data never becomes an executable formula.
fn xlsx(options: &TransferOptions, data: &TransferData) -> Result<Vec<u8>, String> {
    let mut rows = vec![
        data.columns
            .iter()
            .cloned()
            .map(Cell::Text)
            .collect::<Vec<_>>(),
    ];
    rows.extend(data.rows.iter().cloned());
    let mut parts=vec![
        ("_rels/.rels".to_owned(),br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_vec()),
        ("xl/worksheets/sheet1.xml".to_owned(),sheet(&rows,options.xlsx_exact_numbers)?.into_bytes()),
    ];
    let query = options.query.as_ref().filter(|_| options.xlsx_query_sheet);
    let mut workbook = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Result 1" sheetId="1" r:id="rId1"/>"#,
    );
    let mut relations = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
    );
    let mut content = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
    );
    if let Some(query) = query {
        workbook.push_str(r#"<sheet name="Query" sheetId="2" r:id="rId2"/>"#);
        relations.push_str(r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>"#);
        content.push_str(r#"<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#);
        parts.push((
            "xl/worksheets/sheet2.xml".into(),
            sheet(&query_chunks(query), true)?.into_bytes(),
        ));
    }
    workbook.push_str("</sheets></workbook>");
    relations.push_str("</Relationships>");
    content.push_str("</Types>");
    parts.push(("xl/workbook.xml".into(), workbook.into_bytes()));
    parts.push(("xl/_rels/workbook.xml.rels".into(), relations.into_bytes()));
    parts.push(("[Content_Types].xml".into(), content.into_bytes()));
    zip_store(&parts)
}
// Excel uses normal binary64 range and at most15 decimal significant digits.
// Values outside a conservative documented range remain exact inline strings.
// https://learn.microsoft.com/office/troubleshoot/excel/floating-point-arithmetic-inaccurate-result
fn excel_numeric(value: &str, exact: bool) -> Result<bool, String> {
    let raw: Box<serde_json::value::RawValue> =
        serde_json::from_str(value).map_err(|_| "Número XLSX não é JSON finito válido")?;
    let value = raw.get();
    if !value.starts_with(['-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9']) {
        return Err("Valor numérico XLSX inválido".into());
    }
    let value = value.strip_prefix('-').unwrap_or(value);
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map_or((value, "0"), |(a, b)| (a, b));
    let Ok(exponent) = exponent.parse::<i64>() else {
        return Ok(false);
    };
    let before_decimal = mantissa.find('.').unwrap_or(mantissa.len()) as i64;
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(char::from)
        .collect::<String>();
    let Some(first) = digits.bytes().position(|digit| digit != b'0') else {
        return Ok((-307..=308).contains(&exponent));
    };
    let significant = &digits[first..];
    if exact && significant.len() > 15 {
        return Ok(false);
    }
    let Some(power) = exponent
        .checked_add(before_decimal)
        .and_then(|power| power.checked_sub(first as i64 + 1))
    else {
        return Ok(false);
    };
    let compare = |limit: &str| {
        significant
            .bytes()
            .chain(std::iter::repeat(b'0'))
            .zip(limit.bytes().chain(std::iter::repeat(b'0')))
            .take(significant.len().max(limit.len()))
            .find_map(|(a, b)| (a != b).then(|| a.cmp(&b)))
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    Ok(match power {
        -307..=307 => true,
        // Safe15-digit bounds inside the documented min/max normal range.
        -308 => compare("222507385850721") != std::cmp::Ordering::Less,
        308 => compare("179769313486231") != std::cmp::Ordering::Greater,
        _ => false,
    })
}
fn query_chunks(query: &str) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    let mut chunk = String::new();
    let mut units = 0;
    for character in query.chars() {
        let size = character.len_utf16();
        if units + size > 32000 {
            rows.push(vec![Cell::Text(std::mem::take(&mut chunk))]);
            units = 0;
        }
        chunk.push(character);
        units += size;
    }
    if !chunk.is_empty() {
        rows.push(vec![Cell::Text(chunk)]);
    }
    rows
}

fn column_name(mut index: usize) -> String {
    let mut letters = Vec::new();
    index += 1;
    while index > 0 {
        index -= 1;
        letters.push((b'A' + (index % 26) as u8) as char);
        index /= 26;
    }
    letters.iter().rev().collect()
}
fn sheet(rows: &[Vec<Cell>], exact: bool) -> Result<String, String> {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (index, row) in rows.iter().enumerate() {
        write!(out, "<row r=\"{}\">", index + 1).unwrap();
        for (column, cell) in row.iter().enumerate() {
            let address = format!("{}{}", column_name(column), index + 1);
            match cell {
                Cell::Null => write!(out, "<c r=\"{address}\"/>").unwrap(),
                Cell::Number(value) if excel_numeric(value, exact)? => {
                    transfer::sql_literal(cell, SqlFlavor::Postgres)?;
                    write!(
                        out,
                        "<c r=\"{address}\" t=\"n\"><v>{}</v></c>",
                        xml_escape(value)?
                    )
                    .unwrap();
                }
                Cell::Bool(value) => write!(
                    out,
                    "<c r=\"{address}\" t=\"b\"><v>{}</v></c>",
                    u8::from(*value)
                )
                .unwrap(),
                _ => {
                    let value = cell.clipboard_text();
                    if value.encode_utf16().count() > 32767 {
                        return Err("Excel limita uma célula a32767 unidades UTF-16; use JSON/CSV para este valor".into());
                    }
                    write!(out,"<c r=\"{address}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",xml_escape(&crate::transfer_xlsx::encode_xstring(&value))?).unwrap();
                }
            }
        }
        out.push_str("</row>");
        if out.len() > transfer::MAX_TRANSFER_FILE {
            return Err("XLSX excede8MiB".into());
        }
    }
    out.push_str("</sheetData></worksheet>");
    Ok(out)
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ ((0u32.wrapping_sub(crc & 1)) & 0xedb8_8320);
        }
    }
    !crc
}
fn zip_store(parts: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in parts {
        let offset = u32::try_from(out.len()).map_err(|_| "Arquivo ZIP grande demais")?;
        let len = u32::try_from(data.len()).map_err(|_| "Parte ZIP grande demais")?;
        let crc = crc32(data);
        let name = name.as_bytes();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        for n in [20u16, 0, 0, 0, 33] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        for n in [crc, len, len] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        for n in [20u16, 20, 0, 0, 0, 33] {
            central.extend_from_slice(&n.to_le_bytes());
        }
        for n in [crc, len, len] {
            central.extend_from_slice(&n.to_le_bytes());
        }
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        for n in [0u16, 0, 0, 0] {
            central.extend_from_slice(&n.to_le_bytes());
        }
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
        if out.len().saturating_add(central.len()) > transfer::MAX_TRANSFER_FILE {
            return Err("XLSX excede8MiB".into());
        }
    }
    let offset = out.len() as u32;
    let size = central.len() as u32;
    out.extend(central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    for n in [0u16, 0, parts.len() as u16, parts.len() as u16] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod numeric_boundary_tests {
    use super::*;
    #[test]
    fn excel_numbers_never_overflow_underflow_or_exceed_exact_precision() {
        for value in [
            "1e-1000",
            "1e999",
            "1e999999999999999999999999999999",
            "9007199254740993",
            "1.2345678901234567",
            "2e-308",
            "1.8e308",
        ] {
            assert!(!excel_numeric(value, true).unwrap(), "{value}");
        }
        for value in [
            "0",
            "-0",
            "1",
            "123456789012345",
            "1.25",
            "1e-307",
            "3e-308",
            "1e308",
            "-1e308",
        ] {
            assert!(excel_numeric(value, true).unwrap(), "{value}");
        }
        for value in ["1e-1000", "1e999", "1.8e308", "2e-308"] {
            assert!(!excel_numeric(value, false).unwrap(), "{value}");
        }
        assert!(excel_numeric("1.2345678901234567", false).unwrap());
        assert!(excel_numeric("NaN", true).is_err());
        let xml = sheet(
            &[vec![
                Cell::Number("1e-1000".into()),
                Cell::Number("1e999".into()),
                Cell::Number("1.25".into()),
            ]],
            true,
        )
        .unwrap();
        assert!(
            xml.contains("t=\"inlineStr\"") && xml.contains("1e-1000") && xml.contains("1e999")
        );
        assert!(xml.contains("t=\"n\"><v>1.25</v>"));
    }
    #[test]
    fn excel_text_limit_and_query_chunks_count_utf16_without_splitting_surrogates() {
        assert!(sheet(&[vec![Cell::Text("😀".repeat(16383))]], true).is_ok());
        assert!(sheet(&[vec![Cell::Text("😀".repeat(16384))]], true).is_err());
        let query = "😀".repeat(40000);
        let chunks = query_chunks(&query);
        assert!(
            chunks
                .iter()
                .all(|row| matches!(&row[0],Cell::Text(text)if text.encode_utf16().count()<=32000))
        );
        assert_eq!(
            chunks
                .iter()
                .map(|row| row[0].clipboard_text())
                .collect::<String>(),
            query
        );
        assert!(sheet(&chunks, true).is_ok());
    }
}
