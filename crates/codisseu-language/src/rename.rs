//! Bounded, all-or-nothing translation of rename metadata. No writes occur here.
use crate::{RenameFileEdits, RenameRange, RenameTextEdit, utf16_byte_offset};
use serde_json::Value;
use std::{collections::HashSet, path::Path};
pub(crate) const MAX_RENAME_BYTES: usize = 4 * 1024 * 1024;
pub(crate) fn new_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 1024
        || name.chars().any(char::is_control)
        || name.trim().is_empty()
    {
        return Err("Nome de rename vazio, excessivo ou com controle inválido".into());
    }
    Ok(())
}
fn fields(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("Objeto de rename inválido")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Rename contém operação/campo não suportado".into());
    }
    Ok(())
}
fn position(value: &Value) -> Result<(u32, u32), String> {
    fields(value, &["line", "character"])?;
    let number = |key| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .filter(|n| *n <= i32::MAX as u64)
            .map(|n| n as u32)
            .ok_or_else(|| "Posição UTF-16 de rename inválida".to_string())
    };
    Ok((number("line")?, number("character")?))
}
fn range(value: &Value) -> Result<RenameRange, String> {
    fields(value, &["start", "end"])?;
    let a = position(value.get("start").ok_or("Rename sem início")?)?;
    let b = position(value.get("end").ok_or("Rename sem fim")?)?;
    if a > b {
        return Err("Intervalo de rename invertido".into());
    }
    Ok(RenameRange {
        start_line: a.0,
        start_character: a.1,
        end_line: b.0,
        end_character: b.1,
    })
}
pub(crate) fn validate_range(text: &str, edit: &RenameRange) -> Result<(), String> {
    utf16_byte_offset(text, edit.start_line, edit.start_character)
        .ok_or("Início de rename fora do buffer UTF-16")?;
    utf16_byte_offset(text, edit.end_line, edit.end_character)
        .ok_or("Fim de rename fora do buffer UTF-16")?;
    Ok(())
}
pub(crate) fn prepare(
    result: Option<&Value>,
    text: &str,
    line: u32,
    character: u32,
) -> Result<(Option<RenameRange>, Option<String>), String> {
    let value = result
        .filter(|v| !v.is_null())
        .ok_or("O símbolo nesta posição não pode ser renomeado")?;
    if let Some(default) = value.get("defaultBehavior") {
        fields(value, &["defaultBehavior"])?;
        return if default == &Value::Bool(true) {
            Ok((None, None))
        } else {
            Err("Provider recusou o comportamento padrão de rename".into())
        };
    }
    let (parsed, placeholder) = if let Some(r) = value.get("range") {
        fields(value, &["range", "placeholder"])?;
        let text = value
            .get("placeholder")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 4096 && !s.contains('\0'))
            .ok_or("Placeholder de rename inválido")?;
        (range(r)?, Some(text.to_owned()))
    } else {
        (range(value)?, None)
    };
    validate_range(text, &parsed)?;
    let a = (parsed.start_line, parsed.start_character);
    let b = (parsed.end_line, parsed.end_character);
    if a == b || (line, character) < a || (line, character) > b {
        return Err("Intervalo de preparação não contém o símbolo solicitado".into());
    }
    Ok((Some(parsed), placeholder))
}
fn edits(
    value: &Value,
    total: &mut usize,
    bytes: &mut usize,
) -> Result<Vec<RenameTextEdit>, String> {
    let array = value
        .as_array()
        .ok_or("Lista de edições de rename inválida")?;
    if array.is_empty() {
        return Err("Rename contém arquivo sem edições".into());
    }
    *total += array.len();
    if *total > 1024 {
        return Err("Rename excede limite de 1024 edições".into());
    }
    let mut result = Vec::with_capacity(array.len());
    for value in array {
        fields(value, &["range", "newText"])?;
        let r = range(value.get("range").ok_or("Edição de rename sem range")?)?;
        let text = value
            .get("newText")
            .and_then(Value::as_str)
            .filter(|s| !s.contains('\0'))
            .ok_or("Texto de edição de rename inválido")?;
        *bytes += text.len() + 32;
        if *bytes > MAX_RENAME_BYTES {
            return Err("Rename normalizado excede 4 MiB".into());
        }
        result.push(RenameTextEdit {
            start_line: r.start_line,
            start_character: r.start_character,
            end_line: r.end_line,
            end_character: r.end_character,
            new_text: text.to_owned(),
        });
    }
    result.sort_by_key(|e| (e.start_line, e.start_character, e.end_line, e.end_character));
    for pair in result.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if (b.start_line, b.start_character) < (a.end_line, a.end_character)
            || (b.start_line, b.start_character) == (a.start_line, a.start_character)
        {
            return Err("Edições de rename sobrepostas ou ambíguas".into());
        }
    }
    Ok(result)
}
pub(crate) fn workspace_edit(
    result: Option<&Value>,
    root: &Path,
) -> Result<Vec<RenameFileEdits>, String> {
    let value = result
        .filter(|v| !v.is_null())
        .ok_or("Rename não retornou alterações")?;
    fields(value, &["changes", "documentChanges"])?;
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > MAX_RENAME_BYTES {
        return Err("Resposta de rename excede 4 MiB".into());
    }
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    let mut total = 0;
    let mut bytes = 0;
    let mut add = |uri: &str, version: Option<u64>, values: &Value| -> Result<(), String> {
        if files.len() >= 32 {
            return Err("Rename excede limite de 32 arquivos".into());
        }
        let path = crate::definition::path(uri, root)?;
        if !paths.insert(path.clone()) {
            return Err("Rename contém caminhos canônicos duplicados".into());
        }
        bytes += path.as_os_str().len();
        let edits = edits(values, &mut total, &mut bytes)?;
        files.push(RenameFileEdits {
            path,
            version,
            edits,
        });
        Ok(())
    };
    match (value.get("changes"), value.get("documentChanges")) {
        (Some(changes), None) => {
            for (uri, values) in changes
                .as_object()
                .ok_or("WorkspaceEdit.changes inválido")?
            {
                add(uri, None, values)?;
            }
        }
        (None, Some(changes)) => {
            for change in changes
                .as_array()
                .ok_or("WorkspaceEdit.documentChanges inválido")?
            {
                fields(change, &["textDocument", "edits"])?;
                let doc = change
                    .get("textDocument")
                    .ok_or("TextDocumentEdit sem documento")?;
                fields(doc, &["uri", "version"])?;
                let uri = doc
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or("TextDocumentEdit sem URI")?;
                let version = match doc.get("version") {
                    Some(Value::Null) => None,
                    Some(n) => Some(
                        n.as_u64()
                            .filter(|n| *n <= i32::MAX as u64)
                            .ok_or("Versão de rename negativa ou fora do limite integer")?,
                    ),
                    None => return Err("TextDocumentEdit sem versão explícita".into()),
                };
                add(
                    uri,
                    version,
                    change.get("edits").ok_or("TextDocumentEdit sem edições")?,
                )?;
            }
        }
        _ => return Err("WorkspaceEdit deve conter changes OU documentChanges sem mistura".into()),
    }
    if files.is_empty() {
        return Err("Rename não retornou alterações".into());
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    fn r(a: u32, b: u32) -> Value {
        json!({"start":{"line":0,"character":a},"end":{"line":0,"character":b}})
    }
    fn edit(a: u32, b: u32) -> Value {
        json!({"range":r(a,b),"newText":"forecast"})
    }
    #[test]
    fn prepare_forms_preserve_utf16_and_reject_refusal_malformed_and_surrogate() {
        let text = "😀rainfall e\u{301}";
        let (a, p) = prepare(Some(&r(2, 10)), text, 0, 4).unwrap();
        assert_eq!(a.unwrap().start_character, 2);
        assert!(p.is_none());
        let (_, p) = prepare(
            Some(&json!({"range":r(2,10),"placeholder":"rainfall"})),
            text,
            0,
            4,
        )
        .unwrap();
        assert_eq!(p.as_deref(), Some("rainfall"));
        assert_eq!(
            prepare(Some(&json!({"defaultBehavior":true})), text, 0, 4).unwrap(),
            (None, None)
        );
        for value in [
            Value::Null,
            json!({"defaultBehavior":false}),
            r(1, 10),
            r(8, 2),
            r(2, 2),
            r(11, 12),
            json!({"range":r(2,10)}),
            json!({"range":r(2,10),"placeholder":"\0"}),
        ] {
            assert!(prepare(Some(&value), text, 0, 4).is_err(), "{value}");
        }
        assert!(prepare(None, text, 0, 4).is_err());
    }
    #[test]
    fn workspace_forms_versions_and_no_partial_unsupported_edits() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("a.ts");
        fs::write(&path, "rainfall").unwrap();
        let uri = crate::protocol::file_uri(&path).unwrap();
        let normal = json!({"changes":{&uri:[edit(0,8)]}});
        let out = workspace_edit(Some(&normal), d.path()).unwrap();
        assert_eq!(out[0].version, None);
        assert_eq!(out[0].edits[0].new_text, "forecast");
        for version in [Value::Null, json!(0), json!(2)] {
            let doc = json!({"documentChanges":[{"textDocument":{"uri":uri,"version":version},"edits":[edit(0,8)]}]});
            let out = workspace_edit(Some(&doc), d.path()).unwrap();
            assert_eq!(out[0].version, version.as_u64());
        }
        for value in [
            Value::Null,
            json!({}),
            json!({"changes":{}}),
            json!({"changes":{&uri:[]}}),
            json!({"changes":{&uri:[edit(0,8)]},"documentChanges":[]}),
            json!({"changeAnnotations":{},"changes":{&uri:[edit(0,8)]}}),
            json!({"documentChanges":[{"kind":"rename","oldUri":uri,"newUri":uri}]}),
            json!({"documentChanges":[{"textDocument":{"uri":uri,"version":-1},"edits":[edit(0,8)]}]}),
            json!({"changes":{&uri:[edit(0,8),edit(2,4)]}}),
            json!({"changes":{&uri:[{"range":r(0,8),"newText":"x","annotationId":"confirm"}]}}),
            json!({"changes":{&uri:[{"range":r(0,8),"newText":"\0"}]}}),
        ] {
            assert!(workspace_edit(Some(&value), d.path()).is_err(), "{value}");
        }
    }
    #[test]
    fn workspace_bounds_paths_symlinks_and_ambiguous_aliases_are_all_or_nothing() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.ts");
        fs::write(&p, "rainfall").unwrap();
        let uri = crate::protocol::file_uri(&p).unwrap();
        let too_many = json!({"changes":{&uri:vec![edit(0,1);1025]}});
        assert!(
            workspace_edit(Some(&too_many), d.path())
                .unwrap_err()
                .contains("1024")
        );
        let too_large =
            json!({"changes":{&uri:[{"range":r(0,1),"newText":"x".repeat(MAX_RENAME_BYTES)}]}});
        assert!(
            workspace_edit(Some(&too_large), d.path())
                .unwrap_err()
                .contains("4 MiB")
        );
        let mut changes = serde_json::Map::new();
        for n in 0..33 {
            let f = d.path().join(format!("{n}.ts"));
            fs::write(&f, "rainfall").unwrap();
            changes.insert(crate::protocol::file_uri(&f).unwrap(), json!([edit(0, 8)]));
        }
        assert!(
            workspace_edit(Some(&json!({"changes":changes})), d.path())
                .unwrap_err()
                .contains("32 arquivos")
        );
        let outside = tempfile::tempdir().unwrap();
        let f = outside.path().join("out.ts");
        fs::write(&f, "rainfall").unwrap();
        let external = crate::protocol::file_uri(&f).unwrap();
        for invalid in [
            external,
            "https://example.invalid/a".into(),
            "file://remote/tmp/a.ts".into(),
            format!("{uri}?q"),
        ] {
            assert!(
                workspace_edit(
                    Some(&json!({"changes":{&uri:[edit(0,8)],invalid:[edit(0,8)]}})),
                    d.path()
                )
                .is_err()
            );
        }
        #[cfg(unix)]
        {
            let link = d.path().join("link.ts");
            std::os::unix::fs::symlink(&p, &link).unwrap();
            let alias = crate::protocol::file_uri(&link).unwrap();
            assert!(
                workspace_edit(
                    Some(&json!({"changes":{&uri:[edit(0,8)],alias:[edit(0,8)]}})),
                    d.path()
                )
                .unwrap_err()
                .contains("duplicados")
            );
        }
    }
}
