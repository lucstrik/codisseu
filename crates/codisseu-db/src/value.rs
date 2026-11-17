//! Presentation of rusqlite's public dynamic-value type.
pub use rusqlite::types::Value;

pub fn value_type_name(value: &Value) -> &'static str {
    use rusqlite::types::Type;
    match value.data_type() {
        Type::Null => "NULL",
        Type::Integer => "INTEGER",
        Type::Real => "REAL",
        Type::Text => "TEXT",
        Type::Blob => "BLOB",
    }
}

/// A bounded label; the underlying value always retains all accepted bytes.
pub fn display_value(value: &Value) -> String {
    match value {
        Value::Text(text) if text.is_empty() => "\"\"".into(),
        Value::Text(text) => {
            text.chars().take(512).collect::<String>()
                + if text.chars().count() > 512 {
                    "…"
                } else {
                    ""
                }
        }
        Value::Blob(bytes) => {
            let preview = bytes
                .iter()
                .take(32)
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>();
            format!(
                "BLOB[{}] X'{}{}'",
                bytes.len(),
                preview,
                if bytes.len() > 32 { "…" } else { "" }
            )
        }
        Value::Null => "NULL".into(),
        Value::Integer(number) => number.to_string(),
        Value::Real(number) => number.to_string(),
    }
}
