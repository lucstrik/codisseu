//! Inlay labels are inert text; positions refer only to the unmodified buffer.
use crate::{InlayHint, RenameRange};
use serde_json::Value;
fn bounded_text(value: &Value, limit: usize) -> Result<String, String> {
    value
        .as_str()
        .filter(|s| s.len() <= limit && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or("Texto inlay inválido ou excessivo".into())
}
fn tooltip(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= 4096 && !s.contains('\0') => Ok(Some(s.clone())),
        Some(v) if v.is_object() => {
            let kind = v
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("Tooltip inlay sem formato")?;
            if !matches!(kind, "plaintext" | "markdown") {
                return Err("Formato de tooltip inlay inválido".into());
            }
            let text = v
                .get("value")
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 4096 && !s.contains('\0'))
                .ok_or("Tooltip inlay excessivo")?;
            Ok(Some(text.into()))
        }
        _ => Err("Tooltip inlay inválido".into()),
    }
}
pub(crate) fn decode(
    value: Option<&Value>,
    text: &str,
    range: &RenameRange,
) -> Result<(Vec<InlayHint>, bool), String> {
    let empty = vec![];
    let values = match value {
        Some(Value::Null) => &empty,
        Some(Value::Array(v)) => v,
        _ => return Err("Inlay hints exigem array ou null".into()),
    };
    let truncated = values.len() > 2000;
    let mut result = vec![];
    let mut total = 0;
    for value in values.iter().take(2000) {
        let p = value.get("position").ok_or("Inlay hint sem posição")?;
        let num = |field| {
            p.get(field)
                .and_then(Value::as_u64)
                .filter(|n| *n <= i32::MAX as u64)
                .map(|n| n as u32)
                .ok_or("Posição inlay inválida")
        };
        let line = num("line")?;
        let character = num("character")?;
        if (line, character) < (range.start_line, range.start_character)
            || (line, character) > (range.end_line, range.end_character)
            || crate::utf16_byte_offset(text, line, character).is_none()
        {
            return Err("Inlay hint fora da faixa ou posição UTF-16 inválida".into());
        }
        let label = match value.get("label") {
            Some(Value::Array(parts)) if parts.len() <= 32 => {
                let mut label = String::new();
                for part in parts {
                    label.push_str(&bounded_text(
                        part.get("value").ok_or("Parte inlay sem texto")?,
                        256,
                    )?);
                }
                label
            }
            Some(v) => bounded_text(v, 512)?,
            None => return Err("Inlay hint sem label".into()),
        };
        if label.is_empty() || label.len() > 512 {
            return Err("Label inlay vazio ou excessivo".into());
        }
        let kind = match value.get("kind") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                v.as_u64()
                    .filter(|n| matches!(n, 1 | 2))
                    .ok_or("Kind inlay inválido")? as u32,
            ),
        };
        let padding = |key| match value.get(key) {
            None => Ok(false),
            Some(v) => v.as_bool().ok_or("Padding inlay inválido"),
        };
        let tooltip = tooltip(value.get("tooltip"))?;
        total += label.len() + tooltip.as_ref().map_or(0, String::len) + 32;
        if total > 1024 * 1024 {
            return Err("Inlay hints excedem1MiB".into());
        }
        result.push(InlayHint {
            line,
            character,
            label,
            kind,
            padding_left: padding("paddingLeft")?,
            padding_right: padding("paddingRight")?,
            tooltip,
        });
    }
    result.sort_by_key(|h| (h.line, h.character));
    Ok((result, truncated))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn labels_parts_links_commands_and_utf16_remain_inert_and_bounded() {
        let text = "let emoji = \"😀\"; let number = 42;\r\n";
        let range = RenameRange {
            start_line: 0,
            start_character: 0,
            end_line: 1,
            end_character: 0,
        };
        let value = json!([{"position":{"line":0,"character":28},"kind":1,"label":[{"value":": i32","command":{"command":"evil"},"location":{"uri":"https://evil"}}],"tooltip":{"kind":"markdown","value":"[click](https://example.invalid)"},"paddingLeft":true}]);
        let (h, t) = decode(Some(&value), text, &range).unwrap();
        assert!(!t);
        assert_eq!(h[0].label, ": i32");
        assert!(h[0].padding_left);
        let bad = json!([{"position":{"line":0,"character":14},"label":":bad"}]);
        assert!(decode(Some(&bad), text, &range).is_err());
        let bad = json!([{"position":{"line":0,"character":0},"label":"\nnot inline"}]);
        assert!(decode(Some(&bad), text, &range).is_err());
        let large = Value::Array(
            (0..2001)
                .map(|_| json!({"position":{"line":0,"character":0},"label":": i32"}))
                .collect(),
        );
        let (h, t) = decode(Some(&large), text, &range).unwrap();
        assert_eq!(h.len(), 2000);
        assert!(t);
    }
}
