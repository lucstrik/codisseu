//! Bounded snapshot transfer. No implicit database scan or query execution.
use crate::remote::{Cell, RemoteResult, TableRef};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

pub const MAX_TRANSFER_ROWS: usize = 500;
pub const MAX_TRANSFER_COLUMNS: usize = 256;
pub const MAX_TRANSFER_CELL: usize = 64 * 1024;
pub const MAX_TRANSFER_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TRANSFER_FILE: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferFormat {
    Csv,
    Tsv,
    Json,
    Ndjson,
    CodisseuJson,
    CodisseuNdjson,
    SqlInsert,
    Extra(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqlFlavor {
    Postgres,
    Mysql,
    Sqlite,
}
#[derive(Clone, Debug)]
pub struct TransferOptions {
    pub format: TransferFormat,
    pub header: bool,
    pub null_marker: String,
    pub spreadsheet_safe: bool,
    pub sql_flavor: SqlFlavor,
    pub sql_table: Option<TableRef>,
    pub sql_key_columns: Vec<String>,
    pub delimiter: Option<u8>,
    pub transpose: bool,
    pub row_header: bool,
    pub query: Option<String>,
    pub xlsx_query_sheet: bool,
    pub xlsx_exact_numbers: bool,
}
impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            format: TransferFormat::Csv,
            header: true,
            null_marker: r"\N".into(),
            spreadsheet_safe: false,
            sql_flavor: SqlFlavor::Postgres,
            sql_table: None,
            sql_key_columns: Vec::new(),
            delimiter: None,
            transpose: false,
            row_header: false,
            query: None,
            xlsx_query_sheet: true,
            xlsx_exact_numbers: true,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferData {
    #[serde(deserialize_with = "decode_columns")]
    pub columns: Vec<String>,
    #[serde(deserialize_with = "decode_rows")]
    pub rows: Vec<Vec<Cell>>,
    pub source_truncated: bool,
}
fn bounded_vec<'de, D, T, const N: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T, const N: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const N: usize> serde::de::Visitor<'de> for Bounded<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "array limitado a {N} elementos")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element()? {
                if values.len() >= N {
                    return Err(serde::de::Error::custom(
                        "Array excede orçamento de transferência",
                    ));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Bounded::<T, N>(std::marker::PhantomData))
}
struct BoundedRow(Vec<Cell>);
impl<'de> Deserialize<'de> for BoundedRow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        bounded_vec::<D, Cell, MAX_TRANSFER_COLUMNS>(d).map(Self)
    }
}
fn decode_columns<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    bounded_vec::<D, String, MAX_TRANSFER_COLUMNS>(d)
}
fn decode_rows<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Vec<Cell>>, D::Error> {
    Ok(bounded_vec::<D, BoundedRow, MAX_TRANSFER_ROWS>(d)?
        .into_iter()
        .map(|r| r.0)
        .collect())
}
struct JsonRecord(BTreeMap<String, Box<serde_json::value::RawValue>>);
impl<'de> Deserialize<'de> for JsonRecord {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct RecordVisitor;
        impl<'de> serde::de::Visitor<'de> for RecordVisitor {
            type Value = JsonRecord;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("objeto com até 256campos únicos")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut record = BTreeMap::new();
                while let Some((name, value)) =
                    map.next_entry::<String, Box<serde_json::value::RawValue>>()?
                {
                    if record.len() >= MAX_TRANSFER_COLUMNS || record.contains_key(&name) {
                        return Err(serde::de::Error::custom(
                            "JSON com campos duplicados ou mais de 256colunas",
                        ));
                    }
                    if value.get().len() > MAX_TRANSFER_CELL {
                        return Err(serde::de::Error::custom("Célula JSON excede 64KiB"));
                    }
                    record.insert(name, value);
                }
                Ok(JsonRecord(record))
            }
        }
        d.deserialize_map(RecordVisitor)
    }
}
#[derive(Deserialize)]
struct JsonRecords(#[serde(deserialize_with = "decode_json_records")] Vec<JsonRecord>);
fn decode_json_records<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<JsonRecord>, D::Error> {
    bounded_vec::<D, JsonRecord, MAX_TRANSFER_ROWS>(d)
}
impl TransferData {
    pub fn from_remote(result: &RemoteResult) -> Self {
        Self {
            columns: result.columns.clone(),
            rows: result.rows.clone(),
            source_truncated: result.truncated,
        }
    }
    pub fn from_sqlite(result: &crate::QueryResult) -> Self {
        Self {
            columns: result.columns.clone(),
            rows: result
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|value| match value {
                            crate::Value::Null => Cell::Null,
                            crate::Value::Integer(v) => Cell::Number(v.to_string()),
                            crate::Value::Real(v) => Cell::Number(v.to_string()),
                            crate::Value::Text(v) => Cell::Text(v.clone()),
                            crate::Value::Blob(v) => Cell::Binary(v.clone()),
                        })
                        .collect()
                })
                .collect(),
            source_truncated: result.truncated,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.columns.len() > MAX_TRANSFER_COLUMNS || self.rows.len() > MAX_TRANSFER_ROWS {
            return Err(
                "Transferência excede 500 linhas/256 colunas; exporte a página carregada".into(),
            );
        }
        let mut bytes = 0usize;
        for name in &self.columns {
            if name.len() > 1024 || name.contains('\0') {
                return Err("Nome de coluna inválido".into());
            }
            bytes = bytes.saturating_add(name.len());
        }
        for row in &self.rows {
            if row.len() != self.columns.len() {
                return Err("Linha não corresponde ao número de colunas".into());
            }
            for cell in row {
                let size = cell_size(cell);
                if size > MAX_TRANSFER_CELL {
                    return Err("Célula excede 64KiB".into());
                }
                bytes = bytes.saturating_add(size + std::mem::size_of::<Cell>());
                if bytes > MAX_TRANSFER_BYTES {
                    return Err("Transferência excede 4MiB de payload".into());
                }
            }
        }
        Ok(())
    }
}
pub fn cell_size(cell: &Cell) -> usize {
    match cell {
        Cell::Null => 0,
        Cell::Bool(_) => 1,
        Cell::Text(v) | Cell::Number(v) | Cell::Json(v) => v.len(),
        Cell::Binary(v) => v.len(),
    }
}
#[derive(Clone, Debug)]
pub struct TransferRequest<T> {
    pub datasource_id: String,
    pub session_generation: u64,
    pub payload: T,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportTarget {
    SqlTable(TableRef),
    MongoCollection(String),
    RedisKeys,
    MemcachedKeys,
}
#[derive(Clone, Debug)]
pub struct ImportMapping {
    pub source_column: usize,
    pub target_column: String,
}
#[derive(Clone, Debug)]
pub struct ImportRequest {
    pub allow_partial: bool,
    pub target: ImportTarget,
    pub data: TransferData,
    pub mapping: Vec<ImportMapping>,
}
#[derive(Clone, Debug)]
pub struct ImportApproval {
    pub preview_id: u64,
}
#[derive(Clone, Debug)]
pub struct ImportPreview {
    pub preview_id: u64,
    pub rows: usize,
    pub columns: Vec<String>,
    pub description: String,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct ImportReport {
    pub attempted: usize,
    pub applied: usize,
    pub failed: usize,
    pub unknown: usize,
    pub transaction_pending: bool,
    pub atomic: bool,
    pub summary: String,
}
#[derive(Clone, Debug)]
pub struct NoSqlImportRequest {
    pub json: String,
    pub target: crate::transfer_nosql::NoSqlImportTarget,
    pub ttl_policy: crate::transfer_nosql::UnknownTtlPolicy,
}

pub fn map_import(request: &ImportRequest) -> Result<TransferData, String> {
    request.data.validate()?;
    if request.data.source_truncated && !request.allow_partial {
        return Err("Importação de snapshot truncado exige um arquivo completo; não inserir silenciosamente só um prefixo".into());
    }
    if request.data.rows.is_empty()
        || request.mapping.is_empty()
        || request.mapping.len() > MAX_TRANSFER_COLUMNS
    {
        return Err("Escolha dados e mapeie ao menos uma coluna".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for mapping in &request.mapping {
        if mapping.source_column >= request.data.columns.len()
            || mapping.target_column.is_empty()
            || mapping.target_column.len() > 1024
            || mapping.target_column.contains('\0')
            || !names.insert(&mapping.target_column)
        {
            return Err("Mapeamento inválido ou destino duplicado".into());
        }
    }
    let result = TransferData {
        columns: request
            .mapping
            .iter()
            .map(|m| m.target_column.clone())
            .collect(),
        rows: request
            .data
            .rows
            .iter()
            .map(|row| {
                request
                    .mapping
                    .iter()
                    .map(|m| row[m.source_column].clone())
                    .collect()
            })
            .collect(),
        source_truncated: false,
    };
    result.validate()?;
    Ok(result)
}

pub fn encode(options: &TransferOptions, data: &TransferData) -> Result<Vec<u8>, String> {
    data.validate()?;
    let transformed = transform(options, data)?;
    let data = &transformed;
    let output = match &options.format {
        TransferFormat::Csv | TransferFormat::Tsv => encode_delimited(options, data)?,
        TransferFormat::Json => encode_json(data, false)?,
        TransferFormat::Ndjson => encode_json(data, true)?,
        TransferFormat::CodisseuJson => serde_json::to_vec_pretty(&Envelope {
            codisseu_transfer: 1,
            data: data.clone(),
        })
        .map_err(|e| e.to_string())?,
        TransferFormat::CodisseuNdjson => {
            let mut text=serde_json::to_string(&serde_json::json!({"codisseu_transfer":1,"columns":data.columns,"source_truncated":data.source_truncated})).map_err(|e|e.to_string())?;
            text.push('\n');
            for row in &data.rows {
                text.push_str(&serde_json::to_string(row).map_err(|e| e.to_string())?);
                text.push('\n');
            }
            text.into_bytes()
        }
        TransferFormat::SqlInsert => encode_sql(options, data)?.into_bytes(),
        TransferFormat::Extra(id) => crate::transfer_formats_extra::encode(id, options, data)?,
    };
    if output.len() > MAX_TRANSFER_FILE {
        return Err("Arquivo excede 8MiB".into());
    }
    Ok(output)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    codisseu_transfer: u8,
    data: TransferData,
}
pub fn decode(options: &TransferOptions, input: &str) -> Result<TransferData, String> {
    if input.len() > MAX_TRANSFER_FILE {
        return Err("Arquivo excede 8MiB".into());
    }
    let data =
        match &options.format {
            TransferFormat::Csv | TransferFormat::Tsv => decode_delimited(options, input)?,
            TransferFormat::Json => decode_json(input, false)?,
            TransferFormat::Ndjson => decode_json(input, true)?,
            TransferFormat::CodisseuJson => {
                let value: Envelope = serde_json::from_str(input)
                    .map_err(|e| format!("Arquivo Codisseu inválido: {e}"))?;
                if value.codisseu_transfer != 1 {
                    return Err("Versão de transferência não suportada".into());
                }
                value.data
            }
            TransferFormat::CodisseuNdjson => {
                #[derive(Deserialize)]
                struct Header {
                    codisseu_transfer: u8,
                    #[serde(deserialize_with = "decode_columns")]
                    columns: Vec<String>,
                    source_truncated: bool,
                }
                let mut lines = input.lines();
                let header: Header = serde_json::from_str(lines.next().ok_or("Arquivo vazio")?)
                    .map_err(|e| e.to_string())?;
                if header.codisseu_transfer != 1 {
                    return Err("Versão de transferência não suportada".into());
                }
                let mut rows = Vec::new();
                for line in lines {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if rows.len() >= MAX_TRANSFER_ROWS {
                        return Err("Arquivo excede 500 linhas".into());
                    }
                    rows.push(
                        serde_json::from_str::<BoundedRow>(line)
                            .map_err(|e| format!("Linha Codisseu inválida: {e}"))?
                            .0,
                    );
                }
                TransferData {
                    columns: header.columns,
                    rows,
                    source_truncated: header.source_truncated,
                }
            }
            TransferFormat::Extra(id)
                if matches!(id.as_str(), "pipe" | "semicolon" | "csv-scripted") =>
            {
                let mut options = options.clone();
                options.format = TransferFormat::Csv;
                options.delimiter = Some(match id.as_str() {
                    "pipe" => b'|',
                    "semicolon" => b';',
                    _ => b',',
                });
                decode_delimited(&options, input)?
            }
            _ => return Err(
                "Formato disponível somente para exportação; importar scripts/DDL não é permitido"
                    .into(),
            ),
        };
    data.validate()?;
    Ok(data)
}
fn delimiter(options: &TransferOptions) -> u8 {
    options
        .delimiter
        .unwrap_or(if options.format == TransferFormat::Tsv {
            b'\t'
        } else {
            b','
        })
}
fn marker(options: &TransferOptions) -> Result<(), String> {
    if !delimiter(options).is_ascii() || [b'"', b'\n', b'\r', 0].contains(&delimiter(options)) {
        return Err("Separador inválido".into());
    }
    if options.null_marker.is_empty()
        || options.null_marker.len() > 32
        || options
            .null_marker
            .bytes()
            .any(|b| [b'\r', b'\n', b'"', delimiter(options)].contains(&b))
    {
        return Err("Marcador NULL inválido (máximo 32 bytes, sem separador/aspas/newline)".into());
    }
    Ok(())
}
fn csv_field(text: &str, sep: u8, force: bool) -> String {
    if force || text.bytes().any(|b| [sep, b'"', b'\n', b'\r'].contains(&b)) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.into()
    }
}
fn encode_delimited(options: &TransferOptions, data: &TransferData) -> Result<Vec<u8>, String> {
    marker(options)?;
    let sep = delimiter(options);
    let separator = (sep as char).to_string();
    let mut out = String::new();
    if options.header {
        out.push_str(
            &data
                .columns
                .iter()
                .map(|v| csv_field(v, sep, false))
                .collect::<Vec<_>>()
                .join(&separator),
        );
        out.push('\n');
    }
    for row in &data.rows {
        let values = row
            .iter()
            .map(|cell| {
                if matches!(cell, Cell::Null) {
                    return options.null_marker.clone();
                }
                let mut text = cell.clipboard_text();
                if options.spreadsheet_safe && text.trim_start().starts_with(['=', '+', '-', '@']) {
                    text.insert(0, '\'');
                }
                csv_field(&text, sep, text == options.null_marker || text.is_empty())
            })
            .collect::<Vec<_>>();
        out.push_str(&values.join(&separator));
        out.push('\n');
    }
    Ok(out.into_bytes())
}
fn decode_delimited(options: &TransferOptions, input: &str) -> Result<TransferData, String> {
    marker(options)?;
    let sep = delimiter(options);
    let bytes = input.strip_prefix('\u{feff}').unwrap_or(input).as_bytes();
    let mut records: Vec<Vec<(String, bool)>> = Vec::new();
    let mut row = Vec::new();
    let mut field = Vec::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut after_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if in_quotes {
            if byte == b'"' {
                if bytes.get(i + 1) == Some(&b'"') {
                    field.push(b'"');
                    i += 2;
                    continue;
                }
                in_quotes = false;
                after_quote = true;
            } else {
                field.push(byte);
            }
            i += 1;
        } else if byte == sep || byte == b'\n' || byte == b'\r' {
            row.push((
                String::from_utf8(std::mem::take(&mut field)).map_err(|_| "CSV deve ser UTF-8")?,
                quoted,
            ));
            quoted = false;
            after_quote = false;
            if byte != sep {
                records.push(std::mem::take(&mut row));
                if byte == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                if records.len() > MAX_TRANSFER_ROWS + 1 {
                    return Err("Arquivo excede 500 linhas de dados".into());
                }
            }
            i += 1;
        } else if byte == b'"' && field.is_empty() && !after_quote {
            quoted = true;
            in_quotes = true;
            i += 1;
        } else {
            if after_quote || byte == b'"' {
                return Err("Aspas CSV inválidas; use aspas duplas escapadas".into());
            }
            field.push(byte);
            i += 1;
        }
        if field.len() > MAX_TRANSFER_CELL {
            return Err("Campo CSV excede 64KiB".into());
        }
        if row.len() > MAX_TRANSFER_COLUMNS {
            return Err("CSV excede 256 colunas".into());
        }
    }
    if in_quotes {
        return Err("CSV com aspas não encerradas".into());
    }
    if !field.is_empty() || quoted || after_quote || !row.is_empty() {
        row.push((
            String::from_utf8(field).map_err(|_| "CSV deve ser UTF-8")?,
            quoted,
        ));
        records.push(row);
    }
    if records.is_empty() {
        return Err("Arquivo vazio".into());
    }
    let columns = if options.header {
        records.remove(0).into_iter().map(|(v, _)| v).collect()
    } else {
        (0..records[0].len())
            .map(|i| format!("column{}", i + 1))
            .collect()
    };
    let rows = records
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(value, quoted)| {
                    if !quoted && value == options.null_marker {
                        Cell::Null
                    } else {
                        Cell::Text(value)
                    }
                })
                .collect()
        })
        .collect();
    Ok(TransferData {
        columns,
        rows,
        source_truncated: false,
    })
}
fn raw_cell(cell: &Cell) -> Result<String, String> {
    Ok(match cell {
        Cell::Null => "null".into(),
        Cell::Bool(value) => value.to_string(),
        Cell::Number(value) => {
            let raw: Box<serde_json::value::RawValue> =
                serde_json::from_str(value).map_err(|_| "Número não é JSON finito válido")?;
            if !raw
                .get()
                .starts_with(['-', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9'])
            {
                return Err("Valor numérico inválido".into());
            }
            raw.get().into()
        }
        Cell::Text(value) => serde_json::to_string(value).map_err(|e| e.to_string())?,
        Cell::Json(value) => serde_json::from_str::<Box<serde_json::value::RawValue>>(value)
            .map_err(|e| e.to_string())?
            .get()
            .into(),
        Cell::Binary(bytes) => {
            serde_json::to_string(&format!("0x{}", hex(bytes))).map_err(|e| e.to_string())?
        }
    })
}
fn encode_json(data: &TransferData, ndjson: bool) -> Result<Vec<u8>, String> {
    if data
        .columns
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != data.columns.len()
    {
        return Err("JSON padrão requer nomes de coluna únicos; escolha Codisseu JSON para preservar nomes repetidos".into());
    }
    let mut records = Vec::new();
    for row in &data.rows {
        let mut fields = Vec::new();
        for (name, cell) in data.columns.iter().zip(row) {
            fields.push(format!(
                "{}:{}",
                serde_json::to_string(name).map_err(|e| e.to_string())?,
                raw_cell(cell)?
            ));
        }
        records.push(format!("{{{}}}", fields.join(",")));
    }
    Ok(if ndjson {
        format!(
            "{}{}",
            records.join("\n"),
            if records.is_empty() { "" } else { "\n" }
        )
    } else {
        format!("[{}]\n", records.join(",\n"))
    }
    .into_bytes())
}
fn decode_json(input: &str, ndjson: bool) -> Result<TransferData, String> {
    let records: Vec<JsonRecord> = if ndjson {
        let mut rows = Vec::new();
        for line in input.lines().filter(|line| !line.trim().is_empty()) {
            if rows.len() >= MAX_TRANSFER_ROWS {
                return Err("NDJSON excede 500 registros".into());
            }
            rows.push(serde_json::from_str(line).map_err(|e| format!("NDJSON inválido: {e}"))?);
        }
        rows
    } else {
        serde_json::from_str::<JsonRecords>(input)
            .map_err(|e| format!("JSON deve ser array de objetos: {e}"))?
            .0
    };
    if records.len() > MAX_TRANSFER_ROWS {
        return Err("JSON excede 500 registros".into());
    }
    let columns = records
        .first()
        .map(|record| record.0.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut rows = Vec::new();
    for JsonRecord(record) in records {
        if record.keys().ne(columns.iter()) {
            return Err("Objetos JSON precisam ter as mesmas colunas; campos ausentes não viram NULL silenciosamente".into());
        }
        let mut row = Vec::new();
        for value in record.into_values() {
            let raw = value.get();
            row.push(match raw.as_bytes().first() {
                Some(b'"') => Cell::Text(serde_json::from_str(raw).map_err(|e| e.to_string())?),
                Some(b'{') | Some(b'[') => Cell::Json(raw.into()),
                _ if raw == "null" => Cell::Null,
                _ if raw == "true" => Cell::Bool(true),
                _ if raw == "false" => Cell::Bool(false),
                _ => Cell::Number(raw.into()),
            });
        }
        rows.push(row);
    }
    Ok(TransferData {
        columns,
        rows,
        source_truncated: false,
    })
}
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn quote_identifier(name: &str, flavor: SqlFlavor) -> String {
    if flavor == SqlFlavor::Mysql {
        format!("`{}`", name.replace('`', "``"))
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}
pub fn sql_literal(cell: &Cell, flavor: SqlFlavor) -> Result<String, String> {
    Ok(match cell {
        Cell::Null => "NULL".into(),
        Cell::Bool(v) => if *v { "TRUE" } else { "FALSE" }.into(),
        Cell::Number(_) => raw_cell(cell)?,
        Cell::Binary(bytes) => {
            if flavor == SqlFlavor::Postgres {
                format!("decode('{}','hex')", hex(bytes))
            } else {
                format!("X'{}'", hex(bytes))
            }
        }
        Cell::Text(v) | Cell::Json(v) => {
            if flavor == SqlFlavor::Postgres {
                format!("E'{}'", v.replace('\\', "\\\\").replace('\'', "''"))
            } else if flavor == SqlFlavor::Mysql {
                format!("CONVERT(X'{}' USING utf8mb4)", hex(v.as_bytes()))
            } else {
                format!("'{}'", v.replace('\'', "''"))
            }
        }
    })
}
pub fn qualified_table(table: &TableRef, flavor: SqlFlavor) -> String {
    if table.namespace.is_empty() {
        quote_identifier(&table.name, flavor)
    } else {
        format!(
            "{}.{}",
            quote_identifier(&table.namespace, flavor),
            quote_identifier(&table.name, flavor)
        )
    }
}
fn transform(options: &TransferOptions, data: &TransferData) -> Result<TransferData, String> {
    if options.transpose
        && (matches!(options.format, TransferFormat::SqlInsert)
            || matches!(&options.format,TransferFormat::Extra(id) if id.starts_with("sql-") || id=="where-clause"))
    {
        return Err("Transposição não disponível para SQL".into());
    }
    let mut data = data.clone();
    if options.transpose {
        let mut columns = vec!["column".into()];
        columns.extend((1..=data.rows.len()).map(|n| format!("row{n}")));
        let rows = data
            .columns
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let mut row = vec![Cell::Text(name.clone())];
                row.extend(data.rows.iter().map(|row| row[index].clone()));
                row
            })
            .collect();
        data = TransferData {
            columns,
            rows,
            source_truncated: data.source_truncated,
        };
    }
    if options.row_header {
        data.columns.insert(0, "row".into());
        for (index, row) in data.rows.iter_mut().enumerate() {
            row.insert(0, Cell::Number((index + 1).to_string()));
        }
    }
    data.validate()?;
    Ok(data)
}
fn encode_sql(options: &TransferOptions, data: &TransferData) -> Result<String, String> {
    let table = options
        .sql_table
        .as_ref()
        .ok_or("Escolha tabela de destino para SQL INSERT")?;
    if table.name.is_empty() || table.name.contains('\0') || table.namespace.contains('\0') {
        return Err("Tabela SQL inválida".into());
    }
    let table = if table.namespace.is_empty() {
        quote_identifier(&table.name, options.sql_flavor)
    } else {
        format!(
            "{}.{}",
            quote_identifier(&table.namespace, options.sql_flavor),
            quote_identifier(&table.name, options.sql_flavor)
        )
    };
    let columns = data
        .columns
        .iter()
        .map(|name| quote_identifier(name, options.sql_flavor))
        .collect::<Vec<_>>()
        .join(",");
    let mut out =
        String::from("-- Somente linhas carregadas. Nenhum DDL, UPSERT ou COMMIT automático.\n");
    if data.source_truncated {
        out.push_str(
            "-- ATENÇÃO: origem truncada; este arquivo contém apenas o snapshot carregado.\n",
        );
    }
    for row in &data.rows {
        out.push_str(&format!(
            "INSERT INTO {table} ({columns}) VALUES ({});\n",
            row.iter()
                .map(|cell| sql_literal(cell, options.sql_flavor))
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        ));
    }
    Ok(out)
}

#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}
#[derive(Clone, Debug)]
pub struct DirectoryListing {
    pub path: PathBuf,
    pub entries: Vec<DirectoryEntry>,
    pub truncated: bool,
}
pub enum FileCommand {
    ReadXlsx {
        path: PathBuf,
        header: bool,
    },
    Render {
        options: TransferOptions,
        data: TransferData,
    },
    ListDirectory {
        path: PathBuf,
    },
    Read {
        path: PathBuf,
        options: TransferOptions,
    },
    Write {
        path: PathBuf,
        options: TransferOptions,
        data: TransferData,
        overwrite: bool,
    },
    ReadRaw {
        path: PathBuf,
    },
    WriteRaw {
        path: PathBuf,
        text: String,
        overwrite: bool,
    },
    Cancel,
}
#[derive(Clone, Debug)]
pub struct FileReport {
    pub path: PathBuf,
    pub rows: usize,
    pub bytes: usize,
    pub source_truncated: bool,
}
enum FileOutcome {
    Rendered(Vec<u8>),
    Directory(DirectoryListing),
    Data(TransferData),
    Raw(String),
    Written(FileReport),
}
struct FileJob {
    generation: u64,
    command: FileCommand,
}
struct FileReply {
    generation: u64,
    result: Result<FileOutcome, String>,
}
pub struct FileService {
    pub rendered: Option<Vec<u8>>,
    pub directory: Option<DirectoryListing>,
    pub data: Option<TransferData>,
    pub raw_text: Option<String>,
    pub report: Option<FileReport>,
    pub error: Option<String>,
    pub revision: u64,
    pub status: String,
    busy: bool,
    sender: Option<SyncSender<FileJob>>,
    receiver: Receiver<FileReply>,
    generation: Arc<AtomicU64>,
}
impl Default for FileService {
    fn default() -> Self {
        Self::new()
    }
}
impl FileService {
    pub fn new() -> Self {
        let (sender, commands) = mpsc::sync_channel::<FileJob>(1);
        let (replies, receiver) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let epoch = generation.clone();
        std::thread::Builder::new()
            .name("codisseu-transfer-files".into())
            .spawn(move || {
                while let Ok(job) = commands.recv() {
                    let result = file_job(job.command, &epoch, job.generation);
                    if replies
                        .send(FileReply {
                            generation: job.generation,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("Worker de arquivos");
        Self {
            rendered: None,
            directory: None,
            data: None,
            raw_text: None,
            report: None,
            error: None,
            revision: 0,
            status: "Transferência inativa".into(),
            busy: false,
            sender: Some(sender),
            receiver,
            generation,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn dispatch(&mut self, command: FileCommand) -> Result<(), String> {
        if matches!(command, FileCommand::Cancel) {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.status = "Cancelamento de arquivo solicitado".into();
            return Ok(());
        }
        if self.busy {
            return Err("Aguarde a operação de arquivo".into());
        }
        self.sender
            .as_ref()
            .ok_or("Worker encerrado")?
            .try_send(FileJob {
                generation: self.generation.load(Ordering::SeqCst),
                command,
            })
            .map_err(|_| "Fila de arquivos indisponível")?;
        self.busy = true;
        self.rendered = None;
        self.directory = None;
        self.data = None;
        self.raw_text = None;
        self.report = None;
        self.error = None;
        self.status = "Arquivo em processamento…".into();
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let reply = match self.receiver.try_recv() {
                Ok(reply) => reply,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self.busy {
                        self.busy = false;
                        self.error = Some("Worker de arquivo encerrou".into());
                        changed = true;
                    }
                    break;
                }
            };
            changed = true;
            self.busy = false;
            match reply.result {
                Ok(FileOutcome::Written(report)) => {
                    self.status =
                        format!("{} linhas · {} bytes gravados", report.rows, report.bytes);
                    self.report = Some(report);
                    self.revision += 1;
                }
                _ if reply.generation != self.generation.load(Ordering::SeqCst) => {
                    self.error = Some("Leitura de arquivo cancelada".into());
                    self.status = "Leitura cancelada".into();
                }
                Ok(FileOutcome::Rendered(bytes)) => {
                    self.rendered = Some(bytes);
                    self.status = "Prévia renderizada".into();
                    self.revision += 1;
                }
                Ok(FileOutcome::Directory(directory)) => {
                    self.directory = Some(directory);
                    self.status = "Pasta carregada".into();
                    self.revision += 1;
                }
                Ok(FileOutcome::Data(data)) => {
                    self.status = format!(
                        "{} linhas lidas; nenhuma alteração enviada",
                        data.rows.len()
                    );
                    self.data = Some(data);
                    self.revision += 1;
                }
                Ok(FileOutcome::Raw(text)) => {
                    self.raw_text = Some(text);
                    self.status = "Arquivo nativo lido; revise antes de enviar".into();
                    self.revision += 1;
                }
                Err(error) => {
                    self.status = error.clone();
                    self.error = Some(error);
                }
            }
        }
        changed
    }
}
impl Drop for FileService {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.sender.take();
    }
}
fn cancelled(epoch: &AtomicU64, generation: u64) -> Result<(), String> {
    if epoch.load(Ordering::SeqCst) != generation {
        Err("Operação de arquivo cancelada".into())
    } else {
        Ok(())
    }
}
fn read_bytes(path: &PathBuf, epoch: &AtomicU64, generation: u64) -> Result<Vec<u8>, String> {
    if !std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("Selecione arquivo regular".into());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Selecione arquivo regular".into());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 64 * 1024];
    loop {
        cancelled(epoch, generation)?;
        let n = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > MAX_TRANSFER_FILE {
            return Err("Arquivo excede 8MiB".into());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(bytes)
}
fn read_file(path: &PathBuf, epoch: &AtomicU64, generation: u64) -> Result<String, String> {
    String::from_utf8(read_bytes(path, epoch, generation)?)
        .map_err(|_| "Arquivo deve ser UTF-8".into())
}

fn file_job(
    command: FileCommand,
    epoch: &AtomicU64,
    generation: u64,
) -> Result<FileOutcome, String> {
    cancelled(epoch, generation)?;
    match command {
        FileCommand::ReadXlsx { path, header } => {
            let bytes = read_bytes(&path, epoch, generation)?;
            let data = crate::transfer_xlsx::decode(&bytes, header)?;
            cancelled(epoch, generation)?;
            Ok(FileOutcome::Data(data))
        }
        FileCommand::Render { options, data } => {
            let bytes = encode(&options, &data)?;
            cancelled(epoch, generation)?;
            Ok(FileOutcome::Rendered(bytes))
        }
        FileCommand::ListDirectory { path } => {
            let mut entries = Vec::new();
            let mut bytes = 0;
            let mut truncated = false;
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                cancelled(epoch, generation)?;
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                bytes += name.len() + entry.path().as_os_str().len();
                if entries.len() >= 256 || bytes > 1024 * 1024 {
                    truncated = true;
                    break;
                }
                entries.push(DirectoryEntry {
                    path: entry.path(),
                    name,
                    is_dir: entry.file_type().map_err(|e| e.to_string())?.is_dir(),
                });
            }
            entries.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            Ok(FileOutcome::Directory(DirectoryListing {
                path,
                entries,
                truncated,
            }))
        }
        FileCommand::Read { path, options } => {
            let text = read_file(&path, epoch, generation)?;
            let data = decode(&options, &text)?;
            cancelled(epoch, generation)?;
            Ok(FileOutcome::Data(data))
        }
        FileCommand::ReadRaw { path } => Ok(FileOutcome::Raw(read_file(&path, epoch, generation)?)),
        FileCommand::Write {
            path,
            options,
            data,
            overwrite,
        } => {
            let bytes = encode(&options, &data)?;
            atomic_file(
                path,
                bytes,
                data.rows.len(),
                data.source_truncated,
                overwrite,
                epoch,
                generation,
            )
        }
        FileCommand::WriteRaw {
            path,
            text,
            overwrite,
        } => {
            if text.len() > MAX_TRANSFER_FILE {
                return Err("Arquivo excede 8MiB".into());
            }
            atomic_file(
                path,
                text.into_bytes(),
                0,
                false,
                overwrite,
                epoch,
                generation,
            )
        }
        FileCommand::Cancel => Err("Pedido de arquivo inválido".into()),
    }
}
fn atomic_file(
    path: PathBuf,
    bytes: Vec<u8>,
    rows: usize,
    source_truncated: bool,
    overwrite: bool,
    epoch: &AtomicU64,
    generation: u64,
) -> Result<FileOutcome, String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    for chunk in bytes.chunks(64 * 1024) {
        cancelled(epoch, generation)?;
        temporary.write_all(chunk).map_err(|e| e.to_string())?;
    }
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    cancelled(epoch, generation)?;
    if overwrite {
        temporary.persist(&path).map_err(|e| e.to_string())?;
    } else {
        temporary.persist_noclobber(&path).map_err(
            |_| "Destino já existe ou não pode ser criado; confirme substituição explicitamente",
        )?;
    }
    Ok(FileOutcome::Written(FileReport {
        path,
        rows,
        bytes: bytes.len(),
        source_truncated,
    }))
}
