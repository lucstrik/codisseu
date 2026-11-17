//! All-or-nothing decoding of formatting TextEdits for one synchronized buffer.
use crate::{FormattingEdit, FormattingResult, RenameRange};
use serde_json::Value;
use std::path::PathBuf;

pub(crate) fn supported(value: Option<&Value>) -> bool {
    value.is_some_and(|v| v == &Value::Bool(true) || v.is_object())
}
pub(crate) fn decode(
    value: Option<&Value>,
    path: PathBuf,
    version: u64,
    range: Option<RenameRange>,
    text: &str,
) -> Result<FormattingResult, String> {
    let empty = Vec::new();
    let edits = match value {
        None => return Err("Resposta de formatação sem resultado".into()),
        Some(Value::Null) => &empty,
        Some(Value::Array(edits)) if edits.len() <= 8192 => edits,
        _ => return Err("Resposta de formatação deve ser TextEdit[] com até8192 edições".into()),
    };
    let mut starts = vec![(0usize, 0usize)];
    for (scalar, (byte, ch)) in text.char_indices().enumerate() {
        if ch == '\n' {
            starts.push((byte + 1, scalar + 1));
        }
    }
    let offset = |p: &Value| -> Result<usize, String> {
        let object = p.as_object().ok_or("Posição de formatação inválida")?;
        if object.keys().any(|k| k != "line" && k != "character") {
            return Err("Campos de posição de formatação inválidos".into());
        }
        let n = |key| {
            p.get(key)
                .and_then(Value::as_u64)
                .filter(|n| *n <= i32::MAX as u64)
                .ok_or("Posição UTF-16 de formatação inválida")
        };
        let line = n("line")? as usize;
        let col = n("character")? as u32;
        let &(byte, base) = starts
            .get(line)
            .ok_or("Linha de formatação fora do buffer")?;
        let end = starts.get(line + 1).map_or(text.len(), |n| n.0 - 1);
        let line = text[byte..end].trim_end_matches('\r');
        let mut units = 0;
        for (i, ch) in line.chars().enumerate() {
            if units == col {
                return Ok(base + i);
            }
            units += ch.len_utf16() as u32;
            if units > col {
                return Err("Formatação divide um par substituto UTF-16".into());
            }
        }
        if units == col {
            Ok(base + line.chars().count())
        } else {
            Err("Coluna de formatação fora do buffer".into())
        }
    };
    let mut parsed = Vec::with_capacity(edits.len());
    let mut total = 0usize;
    for edit in edits {
        let object = edit.as_object().ok_or("TextEdit de formatação inválido")?;
        if object.keys().any(|k| k != "range" && k != "newText") {
            return Err("Formatação contém campos/operações fora do documento".into());
        }
        let r = edit
            .get("range")
            .and_then(Value::as_object)
            .ok_or("Formatação sem intervalo")?;
        if r.keys().any(|k| k != "start" && k != "end") {
            return Err("Intervalo de formatação inválido".into());
        }
        let start = offset(r.get("start").ok_or("Formatação sem início")?)?;
        let end = offset(r.get("end").ok_or("Formatação sem fim")?)?;
        let new_text = edit
            .get("newText")
            .and_then(Value::as_str)
            .ok_or("Formatação sem novo texto")?;
        total = total
            .checked_add(new_text.len())
            .ok_or("Formatação excessiva")?;
        if start > end || total > 1024 * 1024 || new_text.contains('\0') {
            return Err("Texto/intervalo de formatação inválido ou acima de1MiB".into());
        }
        parsed.push(FormattingEdit {
            range: start..end,
            new_text: new_text.into(),
        });
    }
    parsed.sort_by_key(|e| (e.range.start, e.range.end));
    for pair in parsed.windows(2) {
        if pair[1].range.start < pair[0].range.end || pair[1].range.start == pair[0].range.start {
            return Err("Edições de formatação sobrepostas ou ambíguas".into());
        }
    }
    let mut bytes = text.len();
    // Scalar boundaries are validated above; compute each byte boundary once.
    let boundaries: Vec<_> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()))
        .collect();
    for edit in &parsed {
        bytes = bytes
            .checked_sub(boundaries[edit.range.end] - boundaries[edit.range.start])
            .and_then(|n| n.checked_add(edit.new_text.len()))
            .ok_or("Tamanho de formatação inválido")?;
    }
    if bytes > 1024 * 1024 {
        return Err("Formatação excederia o limite de1MiB do documento".into());
    }
    Ok(FormattingResult {
        path,
        version,
        range,
        edits: parsed,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn e(sl: u32, sc: u32, el: u32, ec: u32, t: &str) -> Value {
        json!({"range":{"start":{"line":sl,"character":sc},"end":{"line":el,"character":ec}},"newText":t})
    }
    #[test]
    fn unicode_crlf_positions_null_and_scope_are_validated_atomically() {
        let text = "// 😀\r\nlet x=1;\r\n";
        let r = decode(
            Some(&json!([e(1, 5, 1, 6, " = ")])),
            "f.ts".into(),
            3,
            None,
            text,
        )
        .unwrap();
        assert_eq!(r.edits[0].range, 11..12);
        assert!(
            decode(
                Some(&json!([e(0, 4, 0, 4, "x")])),
                "f.ts".into(),
                3,
                None,
                text
            )
            .is_err()
        );
        assert!(
            decode(
                Some(&json!([e(1, 5, 1, 7, "x"), e(1, 6, 1, 6, "y")])),
                "f.ts".into(),
                3,
                None,
                text
            )
            .is_err()
        );
        assert!(
            decode(
                Some(&json!({"changes":{"other.ts":[]}})),
                "f.ts".into(),
                3,
                None,
                text
            )
            .is_err()
        );
        assert!(
            decode(Some(&Value::Null), "f.ts".into(), 3, None, text)
                .unwrap()
                .edits
                .is_empty()
        );
    }
}
