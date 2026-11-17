//! Bounded completion decoding. Provider editing instructions are data: no
//! commands, resolve calls, snippets or external content are executed here.
use crate::{CompletionEdit, CompletionItem, CompletionResult, utf16_byte_offset};
use serde_json::Value;
use std::{ops::Range, path::PathBuf};
const MAX_ITEMS: usize = 128;
const MAX_NEW_TEXT: usize = 64 * 1024;
const MAX_ITEM_TEXT: usize = 128 * 1024;
const MAX_RESULT_TEXT: usize = 512 * 1024;
type Position = (u32, u32);
fn position(v: &Value) -> Result<Position, String> {
    let field = |name| {
        v.get(name)
            .and_then(Value::as_u64)
            .filter(|n| *n <= i32::MAX as u64)
            .map(|n| n as u32)
            .ok_or_else(|| "Posição completion inválida".to_string())
    };
    Ok((field("line")?, field("character")?))
}
fn span(v: &Value, text: &str) -> Result<(Position, Position, Range<usize>), String> {
    let start = position(v.get("start").ok_or("Range completion sem início")?)?;
    let end = position(v.get("end").ok_or("Range completion sem fim")?)?;
    if start > end {
        return Err("Range completion invertido".into());
    }
    let begin = utf16_byte_offset(text, start.0, start.1)
        .ok_or("Range completion fora do documento/UTF-16")?;
    let finish =
        utf16_byte_offset(text, end.0, end.1).ok_or("Range completion fora do documento/UTF-16")?;
    Ok((start, end, begin..finish))
}
fn string(v: &Value, max: usize) -> Result<String, String> {
    let s = v.as_str().ok_or("Texto completion inválido")?;
    if s.len() > max || s.contains('\0') {
        return Err("Texto completion excede limite ou contém NUL".into());
    }
    Ok(s.into())
}
fn optional(v: Option<&Value>, max: usize) -> Result<Option<String>, String> {
    v.filter(|v| !v.is_null())
        .map(|v| string(v, max))
        .transpose()
}
fn edit(v: &Value, text: &str) -> Result<CompletionEdit, String> {
    let (start, end, _) = span(v.get("range").ok_or("TextEdit completion sem range")?, text)?;
    Ok(CompletionEdit {
        start_line: start.0,
        start_character: start.1,
        end_line: end.0,
        end_character: end.1,
        new_text: string(
            v.get("newText").ok_or("TextEdit completion sem newText")?,
            MAX_NEW_TEXT,
        )?,
    })
}
fn primary(
    v: &Value,
    new_text: String,
    text: &str,
    cursor: Position,
) -> Result<(CompletionEdit, Option<CompletionEdit>), String> {
    let mut insert_edit = None;
    let range = if let Some(range) = v.get("range") {
        range
    } else {
        let insert = v.get("insert").ok_or("InsertReplaceEdit sem insert")?;
        let replace = v.get("replace").ok_or("InsertReplaceEdit sem replace")?;
        let (a, b, _) = span(insert, text)?;
        let (c, d, _) = span(replace, text)?;
        if a != c || b > d || a.0 != b.0 || c.0 != d.0 || cursor < a || cursor > b {
            return Err("InsertReplaceEdit inválido".into());
        }
        insert_edit = Some(CompletionEdit {
            start_line: a.0,
            start_character: a.1,
            end_line: b.0,
            end_character: b.1,
            new_text: new_text.clone(),
        });
        replace
    };
    let (start, end, _) = span(range, text)?;
    if start.0 != end.0 || cursor < start || cursor > end {
        return Err("TextEdit completion deve conter cursor em uma linha".into());
    }
    Ok((
        CompletionEdit {
            start_line: start.0,
            start_character: start.1,
            end_line: end.0,
            end_character: end.1,
            new_text,
        },
        insert_edit,
    ))
}
fn bytes(e: &CompletionEdit, text: &str) -> Range<usize> {
    utf16_byte_offset(text, e.start_line, e.start_character).unwrap()
        ..utf16_byte_offset(text, e.end_line, e.end_character).unwrap()
}
fn overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    if a.is_empty() {
        b.start <= a.start && a.start <= b.end
    } else if b.is_empty() {
        a.start <= b.start && b.start <= a.end
    } else {
        a.start < b.end && b.start < a.end
    }
}

/// Phpactor 2026.10 emits separate formatting inserts alongside a use import at
/// the very same point. Normalize only those additional zero-width edits into
/// one ordered insertion before the normal strict decoder. Primary edits and
/// nonempty ranges keep their original conflict validation.
pub(crate) fn normalize_phpactor(value: Option<&Value>) -> Option<Value> {
    let mut value = value?.clone();
    let items = if value.is_array() {
        value.as_array_mut()
    } else {
        value.get_mut("items").and_then(Value::as_array_mut)
    }?;
    for item in items.iter_mut().take(MAX_ITEMS) {
        let Some(edits) = item
            .get_mut("additionalTextEdits")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        if edits.len() > 16 {
            continue;
        }
        let mut normalized: Vec<Value> = Vec::new();
        for edit in edits.iter() {
            let at = edit.get("range").filter(|range| {
                range.get("start").is_some() && range.get("start") == range.get("end")
            });
            if let Some(range) = at
                && let Some(new_text) = edit.get("newText").and_then(Value::as_str)
                && new_text.len() <= MAX_NEW_TEXT
                && let Some(previous) = normalized
                    .iter_mut()
                    .find(|e| e.get("range") == Some(range))
                && let Some(before) = previous.get("newText").and_then(Value::as_str)
                && before.len() + new_text.len() <= MAX_NEW_TEXT
            {
                previous["newText"] = Value::String(format!("{before}{new_text}"));
            } else {
                normalized.push(edit.clone());
            }
        }
        *edits = normalized;
    }
    Some(value)
}
fn decode_item(
    v: &Value,
    defaults: Option<&Value>,
    text: &str,
    cursor: Position,
) -> Result<CompletionItem, String> {
    let raw_label = v.get("label").ok_or("CompletionItem sem label")?;
    let label = string(raw_label, 1024)?;
    if label.is_empty() {
        return Err("CompletionItem com label vazio".into());
    }
    let mut item = CompletionItem {
        label,
        detail: None,
        kind: None,
        filter_text: None,
        sort_text: None,
        new_text: String::new(),
        text_edit: None,
        insert_edit: None,
        additional_edits: vec![],
        unsupported_reason: None,
    };
    let parsed = (|| -> Result<(), String> {
        item.detail = optional(v.get("detail"), 4096)?;
        item.filter_text = optional(v.get("filterText"), 1024)?;
        item.sort_text = optional(v.get("sortText"), 1024)?;
        item.kind = v
            .get("kind")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0 && *n <= i32::MAX as u64)
            .map(|n| n as u32);
        let prop = |key| v.get(key).or_else(|| defaults.and_then(|d| d.get(key)));
        if prop("insertTextFormat").is_some_and(|n| n != &Value::from(1)) {
            return Err("Snippets/formato completion não suportados".into());
        }
        if prop("insertTextMode").is_some_and(|n| n != &Value::from(1)) {
            return Err("Ajuste de indentação completion não suportado".into());
        }
        if v.get("command").is_some_and(|c| !c.is_null()) {
            return Err("Completion exige comando não suportado".into());
        }
        let text_edit = v.get("textEdit").filter(|e| !e.is_null());
        if let Some(e) = text_edit {
            item.new_text = string(
                e.get("newText").ok_or("TextEdit completion sem newText")?,
                MAX_NEW_TEXT,
            )?;
            let (replace, insert) = primary(e, item.new_text.clone(), text, cursor)?;
            item.text_edit = Some(replace);
            item.insert_edit = insert;
        } else if let Some(range) = defaults.and_then(|d| d.get("editRange")) {
            let new_text = v
                .get("textEditText")
                .or_else(|| v.get("insertText"))
                .unwrap_or(raw_label);
            item.new_text = string(new_text, MAX_NEW_TEXT)?;
            let wrapped = if range.get("insert").is_some() {
                range.clone()
            } else {
                serde_json::json!({"range":range})
            };
            let (replace, insert) = primary(&wrapped, item.new_text.clone(), text, cursor)?;
            item.text_edit = Some(replace);
            item.insert_edit = insert;
        } else {
            if v.get("textEditText").is_some() {
                return Err("textEditText sem editRange padrão".into());
            }
            item.new_text = string(v.get("insertText").unwrap_or(raw_label), MAX_NEW_TEXT)?;
        }
        if let Some(additional) = v.get("additionalTextEdits").filter(|v| !v.is_null()) {
            let edits = additional
                .as_array()
                .ok_or("additionalTextEdits inválido")?;
            if edits.len() > 16 {
                return Err("Completion excede16 edições adicionais".into());
            }
            for value in edits {
                item.additional_edits.push(edit(value, text)?);
            }
        }
        let mut ranges = Vec::new();
        if let Some(primary) = &item.text_edit {
            ranges.push(bytes(primary, text));
        }
        for extra in &item.additional_edits {
            let range = bytes(extra, text);
            if ranges.iter().any(|r| overlap(r, &range)) {
                return Err("Edições completion sobrepostas".into());
            }
            ranges.push(range);
        }
        if item.new_text.len()
            + item
                .additional_edits
                .iter()
                .map(|e| e.new_text.len())
                .sum::<usize>()
            > MAX_ITEM_TEXT
        {
            return Err("Completion excede128 KiB de texto por item".into());
        }
        Ok(())
    })();
    if let Err(reason) = parsed {
        item.unsupported_reason = Some(reason);
        // An invalid item is display-only; do not expose partially decoded edits
        // as if they were an applicable substitute.
        item.new_text.clear();
        item.text_edit = None;
        item.insert_edit = None;
        item.additional_edits.clear();
    }
    Ok(item)
}
pub(crate) fn decode(
    value: Option<&Value>,
    path: PathBuf,
    version: u64,
    cursor: Position,
    text: &str,
) -> Result<CompletionResult, String> {
    let v = value.ok_or("Resposta completion sem result")?;
    let (values, incomplete, defaults) = if v.is_null() {
        (None, false, None)
    } else if let Some(values) = v.as_array() {
        (Some(values), false, None)
    } else {
        let values = v
            .get("items")
            .and_then(Value::as_array)
            .ok_or("CompletionList sem items")?;
        let incomplete = v
            .get("isIncomplete")
            .and_then(Value::as_bool)
            .ok_or("CompletionList sem isIncomplete válido")?;
        let defaults = v.get("itemDefaults").filter(|v| !v.is_null());
        if let Some(defaults) = defaults {
            let map = defaults.as_object().ok_or("itemDefaults inválido")?;
            if map.keys().any(|k| {
                !matches!(
                    k.as_str(),
                    "editRange" | "insertTextFormat" | "insertTextMode"
                )
            }) {
                return Err("itemDefaults contém propriedade não negociada".into());
            }
        }
        (Some(values), incomplete, defaults)
    };
    let mut result = CompletionResult {
        path,
        version,
        line: cursor.0,
        character: cursor.1,
        items: vec![],
        is_incomplete: incomplete,
        truncated: false,
    };
    let mut size = 0;
    if let Some(values) = values {
        result.truncated = values.len() > MAX_ITEMS;
        for v in values.iter().take(MAX_ITEMS) {
            let item = decode_item(v, defaults, text, cursor)?;
            size += item.new_text.len()
                + item
                    .additional_edits
                    .iter()
                    .map(|e| e.new_text.len())
                    .sum::<usize>();
            if size > MAX_RESULT_TEXT {
                result.truncated = true;
                break;
            }
            result.items.push(item);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn range(a: u32, b: u32) -> Value {
        json!({"start":{"line":0,"character":a},"end":{"line":0,"character":b}})
    }
    fn parse(v: Value) -> CompletionResult {
        decode(Some(&v), "/own.ts".into(), 3, (0, 5), "😀foo").unwrap()
    }
    #[test]
    fn phpactor_import_inserts_are_coalesced_in_original_order_before_strict_decode() {
        let raw = json!([{"label":"forest","insertText":"forest","additionalTextEdits":[
            {"range":range(0,0),"newText":"use Fixture;"},
            {"range":range(0,0),"newText":"\r\n"},
            {"range":range(0,0),"newText":"\r\n"}
        ]}]);
        assert!(parse(raw.clone()).items[0].unsupported_reason.is_some());
        let normalized = normalize_phpactor(Some(&raw)).unwrap();
        let item = &parse(normalized).items[0];
        assert!(item.unsupported_reason.is_none());
        assert_eq!(item.additional_edits.len(), 1);
        assert_eq!(item.additional_edits[0].new_text, "use Fixture;\r\n\r\n");
    }
    #[test]
    fn phpactor_normalization_keeps_primary_overlap_invalid() {
        let raw = json!([{"label":"forest","textEdit":{"range":range(2,5),"newText":"forest"},"additionalTextEdits":[
            {"range":range(2,2),"newText":"first"},
            {"range":range(2,2),"newText":"second"}
        ]}]);
        let item = &parse(normalize_phpactor(Some(&raw)).unwrap()).items[0];
        assert!(item.unsupported_reason.is_some());
    }
    #[test]
    fn list_defaults_replace_and_plaintext_fallback_preserve_unicode() {
        let r = parse(
            json!({"isIncomplete":true,"itemDefaults":{"editRange":{"insert":range(2,5),"replace":range(2,5)},"insertTextFormat":1},"items":[{"label":"forest","textEditText":"forest"}]}),
        );
        assert!(r.is_incomplete);
        assert!(!r.truncated);
        assert_eq!(r.items[0].text_edit.as_ref().unwrap().start_character, 2);
        assert_eq!(r.items[0].new_text, "forest");
        let r = parse(json!([{"label":"foo","insertText":"food"}]));
        assert_eq!(r.items[0].new_text, "food");
        assert!(r.items[0].text_edit.is_none());
        assert!(parse(Value::Null).items.is_empty());
    }
    #[test]
    fn unsafe_items_are_explicit_and_never_expose_partial_edits() {
        for item in [
            json!({"label":"snippet","insertTextFormat":2,"insertText":"foo($1)"}),
            json!({"label":"command","command":{"command":"unsafe"}}),
            json!({"label":"indent","insertTextMode":2}),
            json!({"label":"surrogate","textEdit":{"range":range(1,5),"newText":"bad"}}),
            json!({"label":"outside","textEdit":{"range":range(2,9),"newText":"bad"}}),
            json!({"label":"overlap","textEdit":{"range":range(2,5),"newText":"safe"},"additionalTextEdits":[{"range":range(2,3),"newText":"unsafe"}]}),
            json!({"label":"nul","insertText":"bad\0"}),
        ] {
            let r = parse(json!([item]));
            let item = &r.items[0];
            assert!(item.unsupported_reason.is_some());
            assert!(item.new_text.is_empty());
            assert!(item.additional_edits.is_empty());
            assert!(item.text_edit.is_none());
        }
    }
    #[test]
    fn malformed_lists_and_resource_limits_are_explicit() {
        assert!(decode(Some(&json!({"items":[]})), "/own.ts".into(), 1, (0, 0), "").is_err());
        assert!(
            decode(
                Some(&json!({"isIncomplete":false,"items":[],"itemDefaults":{"data":{}}})),
                "/own.ts".into(),
                1,
                (0, 0),
                ""
            )
            .is_err()
        );
        assert!(parse(json!(vec![json!({"label":"ok"}); 129])).truncated);
        let r = parse(json!([{"label":"large","insertText":"x".repeat(MAX_NEW_TEXT+1)}]));
        assert!(r.items[0].unsupported_reason.is_some());
    }
    #[test]
    fn insert_replace_preserves_distinct_suffix_operations_after_emoji() {
        let text = "😀rainXYZ";
        let value = json!([{"label":"rainfall","textEdit":{"insert":range(2,6),"replace":range(2,9),"newText":"rainfall()"}}]);
        let result = decode(Some(&value), "/own.ts".into(), 1, (0, 6), text).unwrap();
        let item = &result.items[0];
        assert!(item.unsupported_reason.is_none());
        let insert = item.insert_edit.as_ref().expect("retain insert range");
        let replace = item.text_edit.as_ref().unwrap();
        assert_eq!(insert.end_character, 6);
        assert_eq!(replace.end_character, 9);
        let apply = |e: &CompletionEdit| {
            let mut edited = text.to_string();
            edited.replace_range(bytes(e, text), &e.new_text);
            edited
        };
        assert_eq!(apply(insert), "😀rainfall()XYZ");
        assert_eq!(apply(replace), "😀rainfall()");
        let plain =
            json!([{"label":"rainfall","textEdit":{"range":range(2,6),"newText":"rainfall"}}]);
        let result = decode(Some(&plain), "/own.ts".into(), 1, (0, 6), text).unwrap();
        assert!(result.items[0].insert_edit.is_none());
    }
}
