//! Bounded OOXML snapshot importer. ZIP/XML codecs use public MIT libraries;
//! see docs/db-transfer-dependencies.md. No extraction, evaluation, or network access.
use crate::{
    remote::Cell,
    transfer::{
        MAX_TRANSFER_CELL, MAX_TRANSFER_COLUMNS, MAX_TRANSFER_FILE, MAX_TRANSFER_ROWS, TransferData,
    },
};
use quick_xml::{Reader, events::Event};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};

const MAX_PARTS: usize = 128;
const MAX_STRINGS: usize = (MAX_TRANSFER_ROWS + 1) * MAX_TRANSFER_COLUMNS;
#[derive(Debug)]
enum Xml {
    Start(String, BTreeMap<String, String>),
    End(String),
    Text(String),
}
fn walk(input: &str, mut visit: impl FnMut(Xml) -> Result<(), String>) -> Result<(), String> {
    let mut reader = Reader::from_str(input);
    reader.config_mut().expand_empty_elements = true;
    reader.config_mut().check_comments = true;
    let mut depth = 0usize;
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("XML XLSX inválido: {e}"))?
        {
            Event::Start(event) => {
                depth += 1;
                if depth > 64 {
                    return Err("XML XLSX excede profundidade 64".into());
                }
                let name = String::from_utf8(event.local_name().as_ref().to_vec())
                    .map_err(|_| "XML XLSX deve ser UTF-8")?;
                if matches!(
                    name.as_str(),
                    "f" | "formula"
                        | "mergeCell"
                        | "mergeCells"
                        | "hyperlink"
                        | "externalLink"
                        | "oleObject"
                        | "rPh"
                ) {
                    return Err(format!(
                        "XLSX com {name} não suportado; exporte valores simples CSV/JSON"
                    ));
                }
                let mut attributes = BTreeMap::new();
                for attribute in event.attributes() {
                    if attributes.len() >= 32 {
                        return Err("XML XLSX excede32 atributos por elemento".into());
                    }
                    let attribute = attribute.map_err(|e| e.to_string())?;
                    let key = std::str::from_utf8(attribute.key.as_ref())
                        .map_err(|_| "Atributo XLSX inválido")?
                        .to_owned();
                    let value = attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| e.to_string())?
                        .into_owned();
                    if value.len() > MAX_TRANSFER_CELL {
                        return Err("Atributo XLSX excede64 KiB".into());
                    }
                    if attributes.insert(key, value).is_some() {
                        return Err("Atributo XML duplicado".into());
                    }
                }
                visit(Xml::Start(name, attributes))?;
            }
            Event::End(event) => {
                if depth == 0 {
                    return Err("XML XLSX com fechamento inválido".into());
                }
                depth -= 1;
                visit(Xml::End(
                    String::from_utf8(event.local_name().as_ref().to_vec())
                        .map_err(|_| "XML XLSX inválido")?,
                ))?;
            }
            Event::Text(text) => visit(Xml::Text(
                text.decode().map_err(|e| e.to_string())?.into_owned(),
            ))?,
            Event::CData(text) => visit(Xml::Text(
                text.decode().map_err(|e| e.to_string())?.into_owned(),
            ))?,
            Event::GeneralRef(reference) => {
                let value = if let Some(character) =
                    reference.resolve_char_ref().map_err(|e| e.to_string())?
                {
                    character.to_string()
                } else {
                    match reference.decode().map_err(|e| e.to_string())?.as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => return Err("Entidade XML externa/customizada não é permitida".into()),
                    }
                };
                visit(Xml::Text(value))?;
            }
            Event::DocType(_) => {
                return Err("DTD/entidades externas não são permitidas em XLSX".into());
            }
            Event::Decl(declaration) => {
                if declaration.version().map_err(|e| e.to_string())?.as_ref() != b"1.0" {
                    return Err("XLSX requer XML1.0".into());
                }
                if let Some(encoding) = declaration.encoding() {
                    let encoding = encoding.map_err(|e| e.to_string())?;
                    if !encoding.eq_ignore_ascii_case(b"utf-8") {
                        return Err("XML XLSX deve usar UTF-8".into());
                    }
                }
            }
            Event::PI(_) => return Err("Instruções XML não suportadas em XLSX".into()),
            Event::Eof => {
                if depth != 0 {
                    return Err("XML XLSX incompleto".into());
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}
fn escaped_unit(bytes: &[u8]) -> Option<u16> {
    if bytes.len() >= 7
        && bytes[0] == b'_'
        && bytes[1] == b'x'
        && bytes[6] == b'_'
        && bytes[2..6].iter().all(u8::is_ascii_hexdigit)
    {
        std::str::from_utf8(&bytes[2..6])
            .ok()
            .and_then(|s| u16::from_str_radix(s, 16).ok())
    } else {
        None
    }
}
fn decode_xstring(value: &str) -> Result<String, String> {
    let mut units = Vec::new();
    let mut index = 0;
    while index < value.len() {
        if let Some(unit) = escaped_unit(&value.as_bytes()[index..]) {
            units.push(unit);
            index += 7;
        } else {
            let character = value[index..]
                .chars()
                .next()
                .ok_or("Xstring XLSX inválida")?;
            let mut storage = [0; 2];
            units.extend_from_slice(character.encode_utf16(&mut storage));
            index += character.len_utf8();
        }
    }
    String::from_utf16(&units).map_err(|_| "Xstring XLSX contém surrogate UTF-16 isolado".into())
}
/// Protect literal OOXML escape-like text; preserve CR/control characters without invalid XML.
/// https://learn.microsoft.com/openspecs/office_standards/ms-oe376/bd0aa042-434a-4ca7-b25f-4e1fd25a954d
pub(crate) fn encode_xstring(value: &str) -> String {
    let mut result = String::new();
    for (index, character) in value.char_indices() {
        if character == '_' && escaped_unit(&value.as_bytes()[index..]).is_some() {
            result.push_str("_x005F_");
        } else if character == '\r'
            || (character < ' ' && !matches!(character, '\n' | '\t'))
            || matches!(character, '\u{fffe}' | '\u{ffff}')
        {
            result.push_str(&format!("_x{:04X}_", character as u32));
        } else {
            result.push(character);
        }
    }
    result
}

fn append(target: &mut String, value: &str) -> Result<(), String> {
    if target.len() + value.len() > MAX_TRANSFER_CELL {
        return Err("Célula XLSX excede64 KiB".into());
    }
    target.push_str(value);
    Ok(())
}
fn parts(input: &[u8]) -> Result<BTreeMap<String, String>, String> {
    if input.len() > MAX_TRANSFER_FILE {
        return Err("XLSX excede8 MiB".into());
    }
    let eocd = input
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .ok_or("XLSX sem diretório ZIP")?;
    if eocd + 22 > input.len() {
        return Err("Diretório ZIP truncado".into());
    }
    let count = u16::from_le_bytes([input[eocd + 10], input[eocd + 11]]) as usize;
    let comment = u16::from_le_bytes([input[eocd + 20], input[eocd + 21]]) as usize;
    if count > MAX_PARTS
        || eocd + 22 + comment != input.len()
        || input[eocd + 4..eocd + 8] != [0, 0, 0, 0]
    {
        return Err("XLSX requer ZIP simples com até128 partes".into());
    }
    let mut archive =
        zip::ZipArchive::new(Cursor::new(input)).map_err(|e| format!("XLSX/ZIP inválido: {e}"))?;
    if archive.len() > MAX_PARTS {
        return Err("XLSX excede128 partes ZIP".into());
    }
    archive
        .validate_header_consistency()
        .map_err(|e| format!("Cabeçalhos XLSX/ZIP inconsistentes: {e}"))?;
    let mut total = 0usize;
    let mut output = BTreeMap::new();
    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("Parte XLSX inválida: {e}"))?;
        let name = entry
            .name()
            .map_err(|e| format!("Nome de parte XLSX inválido: {e}"))?
            .into_owned();
        if name.len() > 1024
            || entry.enclosed_name().is_none()
            || !names.insert(name.clone())
            || name.contains('\\')
        {
            return Err("Nome de parte XLSX inválido/duplicado".into());
        }
        if name.contains("externalLinks/")
            || name.ends_with("vbaProject.bin")
            || name.contains("embeddings/")
        {
            return Err(
                "XLSX com links externos/macros/objetos incorporados não é permitido".into(),
            );
        }
        if entry.size() > MAX_TRANSFER_FILE.saturating_sub(total) as u64 {
            return Err("XLSX expandido excede8 MiB".into());
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take((MAX_TRANSFER_FILE - total + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        total += bytes.len();
        if total > MAX_TRANSFER_FILE {
            return Err("XLSX expandido excede8 MiB".into());
        }
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let xml = String::from_utf8(bytes).map_err(|_| "XLSX XML deve ser UTF-8")?;
            walk(&xml, |event| {
                if let Xml::Start(tag, attributes) = event
                    && tag == "Relationship"
                    && (attributes
                        .get("TargetMode")
                        .is_some_and(|mode| mode.eq_ignore_ascii_case("External"))
                        || attributes.get("Target").is_some_and(|target| {
                            target.contains("://") || target.starts_with("//")
                        }))
                {
                    return Err("Relacionamento externo XLSX não é permitido".into());
                }
                Ok(())
            })?;
            output.insert(name, xml);
        }
    }
    Ok(output)
}
fn source_sheet(parts: &BTreeMap<String, String>) -> Result<String, String> {
    let workbook = parts
        .get("xl/workbook.xml")
        .ok_or("XLSX sem workbook.xml")?;
    let mut sheets = Vec::new();
    walk(workbook, |event| {
        if let Xml::Start(name, attributes) = event
            && name == "sheet"
        {
            if sheets.len() >= 128 {
                return Err("Workbook excede128 sheets".into());
            }
            let id = attributes
                .get("r:id")
                .or_else(|| {
                    attributes
                        .iter()
                        .find(|(name, _)| name.ends_with(":id"))
                        .map(|(_, value)| value)
                })
                .ok_or("Sheet XLSX sem relacionamento")?
                .clone();
            sheets.push((attributes.get("name").cloned().unwrap_or_default(), id));
        }
        Ok(())
    })?;
    let (_, id) = sheets
        .iter()
        .find(|(name, _)| name.replace(' ', "").eq_ignore_ascii_case("result1"))
        .or_else(|| sheets.first())
        .ok_or("Workbook sem planilha")?;
    let relations = parts
        .get("xl/_rels/workbook.xml.rels")
        .ok_or("XLSX sem relacionamentos do workbook")?;
    let mut target = None;
    walk(relations, |event| {
        if let Xml::Start(name, attributes) = event
            && name == "Relationship"
            && attributes.get("Id") == Some(id)
        {
            if target.is_some() {
                return Err("Relacionamento sheet duplicado".into());
            }
            target = attributes.get("Target").cloned();
        }
        Ok(())
    })?;
    let target = target.ok_or("Sheet sem parte correspondente")?;
    let target = if target.starts_with('/') {
        target.trim_start_matches('/').to_owned()
    } else {
        format!("xl/{target}")
    };
    if target.split('/').any(|part| matches!(part, ".." | "."))
        || !target.starts_with("xl/worksheets/")
        || !target.ends_with(".xml")
    {
        return Err("Caminho de worksheet XLSX não suportado".into());
    }
    Ok(target)
}
fn shared_strings(parts: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let Some(xml) = parts.get("xl/sharedStrings.xml") else {
        return Ok(vec![]);
    };
    let mut values = Vec::new();
    let mut current = None;
    let mut in_text = false;
    walk(xml, |event| {
        match event {
            Xml::Start(name, _) if name == "si" => {
                if current.is_some() || values.len() >= MAX_STRINGS {
                    return Err("SharedStrings XLSX excede orçamento".into());
                }
                current = Some(String::new());
            }
            Xml::Start(name, _) if name == "t" => in_text = true,
            Xml::End(name) if name == "t" => in_text = false,
            Xml::End(name) if name == "si" => values.push(decode_xstring(
                &current.take().ok_or("SharedString XLSX inválida")?,
            )?),
            Xml::Text(text) if in_text => {
                if let Some(current) = &mut current {
                    append(current, &text)?;
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(values)
}
fn date_styles(parts: &BTreeMap<String, String>) -> Result<Vec<bool>, String> {
    let Some(xml) = parts.get("xl/styles.xml") else {
        return Ok(vec![]);
    };
    let mut formats = BTreeSet::new();
    let mut styles = Vec::new();
    let mut in_xfs = false;
    walk(xml, |event| {
        match event {
            Xml::Start(name, attributes) if name == "numFmt" => {
                let id = attributes
                    .get("numFmtId")
                    .and_then(|id| id.parse::<u32>().ok())
                    .ok_or("numFmt XLSX inválido")?;
                let mut quoted = false;
                let mut bracket = false;
                let mut escape = false;
                let mut is_date = false;
                for character in attributes
                    .get("formatCode")
                    .map_or("", String::as_str)
                    .chars()
                {
                    if escape {
                        escape = false;
                        continue;
                    }
                    match character {
                        '\\' => escape = true,
                        '"' => quoted = !quoted,
                        '[' => bracket = true,
                        ']' => bracket = false,
                        _ if !quoted
                            && !bracket
                            && matches!(
                                character.to_ascii_lowercase(),
                                'y' | 'm' | 'd' | 'h' | 's'
                            ) =>
                        {
                            is_date = true
                        }
                        _ => {}
                    }
                }
                if is_date {
                    formats.insert(id);
                }
            }
            Xml::Start(name, _) if name == "cellXfs" => in_xfs = true,
            Xml::End(name) if name == "cellXfs" => in_xfs = false,
            Xml::Start(name, attributes) if name == "xf" && in_xfs => {
                if styles.len() >= 4096 {
                    return Err("XLSX excede4096 estilos".into());
                }
                let id = attributes
                    .get("numFmtId")
                    .and_then(|id| id.parse::<u32>().ok())
                    .unwrap_or(0);
                styles.push(matches!(id,14..=22|27..=36|45..=47|50..=58) || formats.contains(&id));
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(styles)
}
fn address(value: &str) -> Result<(usize, usize), String> {
    let mut column = 0usize;
    let mut split = 0;
    for byte in value.bytes() {
        if byte.is_ascii_uppercase() {
            column = column
                .checked_mul(26)
                .and_then(|c| c.checked_add(usize::from(byte - b'A') + 1))
                .ok_or("Coluna XLSX inválida")?;
            split += 1;
        } else {
            break;
        }
    }
    if split == 0 || column == 0 || column > MAX_TRANSFER_COLUMNS {
        return Err("XLSX excede256 colunas ou endereço inválido".into());
    }
    let row = value[split..]
        .parse::<usize>()
        .map_err(|_| "Linha XLSX inválida")?;
    if row == 0 || row > MAX_TRANSFER_ROWS + 1 {
        return Err("XLSX excede500 linhas de dados".into());
    }
    Ok((row - 1, column - 1))
}
struct PendingCell {
    column: usize,
    kind: String,
    value: String,
    inline: String,
    style: usize,
}
fn finish_cell(cell: PendingCell, strings: &[String], styles: &[bool]) -> Result<Cell, String> {
    Ok(match cell.kind.as_str() {
        "inlineStr" => Cell::Text(decode_xstring(&cell.inline)?),
        "s" => {
            if cell.value.is_empty() {
                Cell::Null
            } else {
                Cell::Text(
                    strings
                        .get(
                            cell.value
                                .parse::<usize>()
                                .map_err(|_| "Índice sharedString inválido")?,
                        )
                        .ok_or("SharedString inexistente")?
                        .clone(),
                )
            }
        }
        "str" | "d" => Cell::Text(decode_xstring(&cell.value)?),
        "b" => match cell.value.as_str() {
            "0" => Cell::Bool(false),
            "1" => Cell::Bool(true),
            "" => Cell::Null,
            _ => return Err("Booleano XLSX inválido".into()),
        },
        "" | "n" => {
            if cell.value.is_empty() {
                Cell::Null
            } else {
                let raw: Box<serde_json::value::RawValue> = serde_json::from_str(&cell.value)
                    .map_err(|_| "Número XLSX inválido/não finito")?;
                if !raw
                    .get()
                    .starts_with(['-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9'])
                {
                    return Err("Número XLSX inválido".into());
                }
                if styles.get(cell.style) == Some(&true) {
                    Cell::Text(cell.value)
                } else {
                    Cell::Number(cell.value)
                }
            }
        }
        "e" => return Err("Célula Excel com erro não pode ser importada silenciosamente".into()),
        _ => return Err(format!("Tipo de célula XLSX não suportado: {}", cell.kind)),
    })
}
/// Reads Result1 when present, otherwise the first declared sheet. No formula evaluation.
pub fn decode(input: &[u8], header: bool) -> Result<TransferData, String> {
    let parts = parts(input)?;
    let source = source_sheet(&parts)?;
    let sheet = parts.get(&source).ok_or("Parte worksheet ausente")?;
    let strings = shared_strings(&parts)?;
    let styles = date_styles(&parts)?;
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut current_row = None;
    let mut current_cell: Option<PendingCell> = None;
    let mut in_value = false;
    let mut in_text = false;
    let mut width = 0usize;
    walk(sheet, |event| {
        match event {
            Xml::Start(name, attributes) if name == "row" => {
                if current_row.is_some() {
                    return Err("Rows XLSX aninhadas".into());
                }
                let index = attributes
                    .get("r")
                    .map(|value| value.parse::<usize>())
                    .transpose()
                    .map_err(|_| "Índice row XLSX inválido")?
                    .unwrap_or(rows.len() + 1);
                if index == 0
                    || index > MAX_TRANSFER_ROWS + usize::from(header)
                    || index <= rows.len()
                {
                    return Err("XLSX excede limite de linhas ou ordem de rows inválida".into());
                }
                while rows.len() < index {
                    rows.push(Vec::new());
                }
                current_row = Some(index - 1);
            }
            Xml::Start(name, attributes) if name == "c" => {
                let row = current_row.ok_or("Cell XLSX fora de row")?;
                if current_cell.is_some() {
                    return Err("Cells XLSX aninhadas".into());
                }
                let column = if let Some(reference) = attributes.get("r") {
                    let (address_row, column) = address(reference)?;
                    if address_row != row {
                        return Err("Endereço cell não corresponde à row XLSX".into());
                    }
                    column
                } else {
                    rows[row].len()
                };
                if column >= MAX_TRANSFER_COLUMNS || column < rows[row].len() {
                    return Err("Cell XLSX duplicada/fora de ordem/limite de colunas".into());
                }
                current_cell = Some(PendingCell {
                    column,
                    kind: attributes.get("t").cloned().unwrap_or_default(),
                    value: String::new(),
                    inline: String::new(),
                    style: attributes
                        .get("s")
                        .map(|s| s.parse::<usize>())
                        .transpose()
                        .map_err(|_| "Estilo XLSX inválido")?
                        .unwrap_or(0),
                });
            }
            Xml::Start(name, _) if name == "v" => in_value = true,
            Xml::End(name) if name == "v" => in_value = false,
            Xml::Start(name, _) if name == "t" => in_text = true,
            Xml::End(name) if name == "t" => in_text = false,
            Xml::Text(text) => {
                if let Some(cell) = &mut current_cell {
                    if in_value {
                        append(&mut cell.value, &text)?;
                    }
                    if in_text {
                        append(&mut cell.inline, &text)?;
                    }
                }
            }
            Xml::End(name) if name == "c" => {
                let cell = current_cell.take().ok_or("Cell XLSX inválida")?;
                let column = cell.column;
                let row = current_row.ok_or("Cell XLSX sem row")?;
                rows[row].resize(column, Cell::Null);
                rows[row].push(finish_cell(cell, &strings, &styles)?);
                width = width.max(column + 1);
            }
            Xml::End(name) if name == "row" => {
                if current_cell.is_some() {
                    return Err("Cell XLSX não encerrada".into());
                }
                current_row = None;
            }
            _ => {}
        }
        Ok(())
    })?;
    if rows.is_empty() || width == 0 {
        return Err("Planilha XLSX vazia".into());
    }
    for row in &mut rows {
        row.resize(width, Cell::Null);
    }
    let columns = if header {
        rows.remove(0)
            .iter()
            .map(|cell| {
                if matches!(cell, Cell::Null) {
                    String::new()
                } else {
                    cell.clipboard_text()
                }
            })
            .collect()
    } else {
        (1..=width).map(|n| format!("column{n}")).collect()
    };
    let data = TransferData {
        columns,
        rows,
        source_truncated: false,
    };
    data.validate()?;
    Ok(data)
}
