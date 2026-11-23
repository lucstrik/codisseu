//! Schema-directed cell input shared by the grid and database adapters.
//! Values remain strings until binding so integer/decimal precision is never lost.
use crate::remote::{Cell, RemoteColumn};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellEditorKind {
    Integer,
    Decimal,
    Float,
    Boolean,
    Json,
    Date,
    Time,
    DateTime,
    Uuid,
    Text,
    Unsupported,
}

/// Base type only; never use this value as executable SQL.
pub fn base_type(column: &RemoteColumn) -> String {
    let kind = column.data_type.trim().to_ascii_lowercase();
    let kind = kind.strip_prefix("generated ").unwrap_or(&kind);
    let output = if let Some((before, after)) = kind.split_once('(') {
        let (_, suffix) = after.rsplit_once(')').unwrap_or((after, ""));
        format!("{before} {suffix}")
    } else {
        kind.to_owned()
    };
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn editor_kind(column: &RemoteColumn) -> CellEditorKind {
    use CellEditorKind::*;
    if column
        .data_type
        .trim()
        .to_ascii_lowercase()
        .starts_with("generated ")
    {
        return Unsupported;
    }
    let kind = base_type(column);
    let kind = kind.strip_suffix(" unsigned").unwrap_or(&kind);
    match kind {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
        | "int8" | "smallserial" | "serial" | "bigserial" => Integer,
        "numeric" | "decimal" | "dec" => Decimal,
        "real" | "float" | "double" | "double precision" | "float4" | "float8" => Float,
        "bool" | "boolean" => Boolean,
        "json" | "jsonb" => Json,
        "date" => Date,
        "time" | "time without time zone" => Time,
        "timestamp"
        | "datetime"
        | "timestamp without time zone"
        | "timestamp with time zone"
        | "timestamptz" => DateTime,
        "uuid" => Uuid,
        "text" | "tinytext" | "mediumtext" | "longtext" | "varchar" | "nvarchar"
        | "character varying" | "char" | "nchar" | "character" | "bpchar" | "enum" | "set" => Text,
        _ => Unsupported,
    }
}

fn dimensions(column: &RemoteColumn) -> Option<Vec<i32>> {
    let (_, rest) = column.data_type.split_once('(')?;
    let (inside, _) = rest.split_once(')')?;
    inside.split(',').map(|v| v.trim().parse().ok()).collect()
}

/// Valid intermediate input: allows empty/sign/exponent prefixes while typing.
/// Complete range, scale and format checks happen again on acceptance and submission.
pub fn accepts_input(column: &RemoteColumn, text: &str) -> bool {
    if text.len() > 64 * 1024 {
        return false;
    }
    match editor_kind(column) {
        CellEditorKind::Integer => {
            let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
            digits.bytes().all(|b| b.is_ascii_digit())
                && !(base_type(column).ends_with(" unsigned") && text.starts_with('-'))
        }
        CellEditorKind::Decimal | CellEditorKind::Float => {
            numeric_input(text)
                && !(base_type(column).ends_with(" unsigned") && text.starts_with('-'))
        }
        CellEditorKind::Unsupported => false,
        _ => true,
    }
}

fn numeric_input(text: &str) -> bool {
    let mut point = false;
    let mut exponent = false;
    let mut mantissa_digits = false;
    let mut previous = None;
    for (i, b) in text.bytes().enumerate() {
        match b {
            b'0'..=b'9' => {
                if !exponent {
                    mantissa_digits = true;
                }
            }
            b'+' | b'-' if i == 0 || matches!(previous, Some(b'e' | b'E')) => {}
            b'.' if !point && !exponent => point = true,
            b'e' | b'E' if !exponent && mantissa_digits => exponent = true,
            _ => return false,
        }
        previous = Some(b);
    }
    true
}

pub fn enum_values(column: &RemoteColumn) -> Option<Vec<String>> {
    if !matches!(base_type(column).as_str(), "enum" | "set") {
        return None;
    }
    let (_, rest) = column.data_type.split_once('(')?;
    let (inside, _) = rest.rsplit_once(')')?;
    let mut chars = inside.chars().peekable();
    let mut values = Vec::new();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.next()? != '\'' {
            return None;
        }
        let mut value = String::new();
        loop {
            match chars.next()? {
                '\'' if chars.peek() == Some(&'\'') => {
                    chars.next();
                    value.push('\'');
                }
                '\'' => break,
                '\\' => value.push(match chars.next()? {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    '0' => '\0',
                    c => c,
                }),
                c => value.push(c),
            }
        }
        values.push(value);
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        match chars.next() {
            None => break,
            Some(',') => {}
            _ => return None,
        }
    }
    Some(values)
}

pub fn parse_cell_edit(column: &RemoteColumn, text: &str, null: bool) -> Result<Cell, String> {
    use CellEditorKind::*;
    let kind = editor_kind(column);
    if kind == Unsupported {
        return Err(format!(
            "{}: tipo {} somente leitura",
            column.name, column.data_type
        ));
    }
    if null {
        return if column.nullable {
            Ok(Cell::Null)
        } else {
            Err(format!("{} não aceita NULL", column.name))
        };
    }
    if text.len() > 64 * 1024 {
        return Err("Célula excede 64 KiB".into());
    }
    if text.contains('\0') {
        return Err("Valor contém caractere NUL não suportado pelo editor SQL".into());
    }
    let invalid = || format!("{}: valor inválido para {}", column.name, column.data_type);
    let value = match kind {
        Integer => {
            if !accepts_input(column, text) || text.is_empty() {
                return Err(invalid());
            }
            let value = text.parse::<i128>().map_err(|_| invalid())?;
            let base = base_type(column);
            let unsigned = base.ends_with(" unsigned");
            let bits = match base.split_whitespace().next().unwrap_or("") {
                "tinyint" => 8,
                "smallint" | "int2" | "smallserial" => 16,
                "mediumint" => 24,
                "int" | "integer" | "int4" | "serial" => 32,
                _ => 64,
            };
            let (min, max) = if unsigned {
                (0, (1_i128 << bits) - 1)
            } else {
                (-(1_i128 << (bits - 1)), (1_i128 << (bits - 1)) - 1)
            };
            if !(min..=max).contains(&value) {
                return Err(format!("{}: limite de {} a {}", column.name, min, max));
            }
            Cell::Number(value.to_string())
        }
        Decimal => {
            validate_decimal(column, text)?;
            Cell::Number(text.into())
        }
        Float => {
            if !numeric_input(text) || !text.parse::<f64>().is_ok_and(f64::is_finite) {
                return Err(invalid());
            }
            let significant = text
                .split(['e', 'E'])
                .next()
                .unwrap_or("")
                .bytes()
                .any(|b| matches!(b, b'1'..=b'9'));
            if significant && text.parse::<f64>().is_ok_and(|v| v == 0.) {
                return Err(invalid());
            }
            let base = base_type(column);
            let base = base.strip_suffix(" unsigned").unwrap_or(&base);
            if matches!(base, "real" | "float" | "float4")
                && (!text.parse::<f32>().is_ok_and(f32::is_finite)
                    || (significant && text.parse::<f32>().is_ok_and(|v| v == 0.)))
            {
                return Err(invalid());
            }
            Cell::Number(text.into())
        }
        Boolean => match text {
            "true" | "1" => Cell::Bool(true),
            "false" | "0" => Cell::Bool(false),
            _ => return Err(invalid()),
        },
        Json => {
            serde_json::from_str::<Box<serde_json::value::RawValue>>(text)
                .map_err(|_| invalid())?;
            Cell::Json(text.into())
        }
        Date => {
            sqlx_core::types::chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map_err(|_| invalid())?;
            Cell::Text(text.into())
        }
        Time => {
            validate_temporal_precision(column, text)?;
            validate_time(column, text).map_err(|_| invalid())?;
            Cell::Text(text.into())
        }
        DateTime => {
            validate_temporal_precision(column, text)?;
            let base = base_type(column);
            if matches!(base.as_str(), "timestamptz" | "timestamp with time zone") {
                let value = text.replacen(' ', "T", 1);
                sqlx_core::types::chrono::DateTime::parse_from_rfc3339(&value)
                    .map_err(|_| invalid())?;
            } else {
                let value = text.replacen('T', " ", 1);
                sqlx_core::types::chrono::NaiveDateTime::parse_from_str(
                    &value,
                    "%Y-%m-%d %H:%M:%S%.f",
                )
                .map_err(|_| invalid())?;
            }
            Cell::Text(text.into())
        }
        Uuid => {
            sqlx_core::types::Uuid::parse_str(text).map_err(|_| invalid())?;
            Cell::Text(text.into())
        }
        Text => {
            let base = base_type(column);
            let byte_limit = match base.as_str() {
                "tinytext" => 255,
                "text" => 65535,
                _ => 65536,
            };
            if text.len() > byte_limit {
                return Err(format!("{}: máximo de {} bytes", column.name, byte_limit));
            }
            if matches!(
                base.as_str(),
                "varchar"
                    | "nvarchar"
                    | "character varying"
                    | "char"
                    | "nchar"
                    | "character"
                    | "bpchar"
            ) && let Some(length) = dimensions(column).and_then(|d| d.first().copied())
                && text.chars().count() > usize::try_from(length).unwrap_or(0)
            {
                return Err(format!("{}: máximo de {} caracteres", column.name, length));
            }
            if matches!(base.as_str(), "enum" | "set") {
                let choices = enum_values(column).ok_or_else(invalid)?;
                let valid = if base == "enum" {
                    choices.iter().any(|v| v == text)
                } else {
                    text.is_empty()
                        || text
                            .split(',')
                            .all(|part| choices.iter().any(|v| v == part))
                };
                if !valid {
                    return Err(format!("{}: selecione um valor permitido", column.name));
                }
            }
            Cell::Text(text.into())
        }
        Unsupported => unreachable!(),
    };
    Ok(value)
}

fn validate_temporal_precision(column: &RemoteColumn, text: &str) -> Result<(), String> {
    let base = base_type(column);
    let default_precision = if matches!(base.as_str(), "datetime" | "timestamp" | "time") {
        0
    } else {
        6
    };
    let precision = dimensions(column)
        .and_then(|d| d.first().copied())
        .unwrap_or(default_precision);
    let fraction = text
        .split_once('.')
        .map(|(_, rest)| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .unwrap_or_default();
    // Trailing zeroes beyond the declared scale are exact, rather than rounding.
    if precision < 0 || fraction.trim_end_matches('0').len() > precision as usize {
        return Err(format!(
            "{}: máximo de {} casas de segundos",
            column.name, precision
        ));
    }
    Ok(())
}

fn validate_time(column: &RemoteColumn, text: &str) -> Result<(), ()> {
    if base_type(column) == "time without time zone" {
        return sqlx_core::types::chrono::NaiveTime::parse_from_str(text, "%H:%M:%S%.f")
            .map(|_| ())
            .map_err(|_| ());
    }
    let text = text.strip_prefix('-').unwrap_or(text);
    let mut parts = text.split(':');
    let hours: u16 = parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let minutes: u8 = parts.next().ok_or(())?.parse().map_err(|_| ())?;
    let seconds = parts.next().ok_or(())?;
    if parts.next().is_some() || hours > 838 || minutes > 59 {
        return Err(());
    }
    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let seconds: u8 = seconds.parse().map_err(|_| ())?;
    if seconds > 59 || fraction.len() > 6 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    Ok(())
}

fn validate_decimal(column: &RemoteColumn, text: &str) -> Result<(), String> {
    let invalid = || {
        format!(
            "{}: decimal inválido ou fora da precisão de {}",
            column.name, column.data_type
        )
    };
    if !numeric_input(text) {
        return Err(invalid());
    }
    let value = text.strip_prefix(['+', '-']).unwrap_or(text);
    let mut exp = value.split(['e', 'E']);
    let mantissa = exp.next().ok_or_else(invalid)?;
    let exponent = exp
        .next()
        .map(|e| e.parse::<i32>())
        .transpose()
        .map_err(|_| invalid())?
        .unwrap_or(0);
    if exp.next().is_some() || exponent.unsigned_abs() > 65536 {
        return Err(invalid());
    }
    let (integer, fractional) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() && fractional.is_empty() {
        return Err(invalid());
    }
    if !integer
        .bytes()
        .chain(fractional.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    if base_type(column).ends_with(" unsigned") && text.starts_with('-') {
        return Err(invalid());
    }
    let decimal_point = integer.len() as i32 + exponent;
    if decimal_point > 65536 || (fractional.len() as i32 - exponent) > 16383 {
        return Err(invalid());
    }
    if let Some(dims) = dimensions(column) {
        let precision = *dims.first().ok_or_else(invalid)?;
        let scale = dims.get(1).copied().unwrap_or(0);
        if !(1..=65536).contains(&precision) || !(-65536..=65536).contains(&scale) {
            return Err(invalid());
        }
        let digits = format!("{integer}{fractional}");
        if let Some(first) = digits.bytes().position(|b| b != b'0') {
            let last = digits.bytes().rposition(|b| b != b'0').unwrap();
            let point = integer.len() as i32 + exponent;
            let whole_digits = point - first as i32;
            let last_power = point - last as i32 - 1;
            if precision <= 0 || whole_digits > precision - scale || last_power < -scale {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

/// Validate again at the worker boundary; GUI validation is never authoritative.
pub fn validate_cell_edit(column: &RemoteColumn, value: &Cell) -> Result<Cell, String> {
    match value {
        Cell::Null => parse_cell_edit(column, "", true),
        Cell::Text(v) | Cell::Number(v) | Cell::Json(v) => parse_cell_edit(column, v, false),
        Cell::Bool(v) => parse_cell_edit(
            column,
            if editor_kind(column) == CellEditorKind::Integer {
                if *v { "1" } else { "0" }
            } else if *v {
                "true"
            } else {
                "false"
            },
            false,
        ),
        Cell::Binary(_) => Err("Célula binária somente leitura".into()),
    }
}
