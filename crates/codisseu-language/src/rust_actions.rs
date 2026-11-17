//! Inert, bounded CodeAction metadata; commands are never executed.
use crate::{CodeActionItem, RenameFileEdits};
use serde_json::Value;
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
pub(crate) struct Action {
    pub item: CodeActionItem,
    pub original: Value,
}
/// The installed typescript-language-server emits an auxiliary command even for
/// edit-only fixes. In its audited protocol this invokes action.commands only.
/// Discard that wrapper only when it is demonstrably empty; never execute it.
pub(crate) fn edit_only_typescript_action(value: &Value) -> Value {
    let mut value = value.clone();
    let safe = value.get("edit").is_some_and(|v| !v.is_null())
        && value.pointer("/command/command").and_then(Value::as_str)
            == Some("_typescript.applyCodeActionCommand")
        && value
            .pointer("/command/arguments")
            .and_then(Value::as_array)
            .is_some_and(|a| {
                a.len() == 1
                    && a[0].get("action").is_some_and(Value::is_object)
                    && a[0]
                        .pointer("/action/commands")
                        .is_none_or(|c| c.is_null() || c.as_array().is_some_and(Vec::is_empty))
            });
    if safe {
        value.as_object_mut().unwrap().remove("command");
    }
    value
}
/// Phpactor's undefined-variable provider returns fixes for the whole document.
/// Narrow concrete local ranges to the requested context, without guessing names.
pub(crate) fn phpactor_contextual(
    value: &Value,
    path: &Path,
    requested: &crate::RenameRange,
) -> bool {
    let overlap = |r: &Value| -> Option<bool> {
        let position =
            |part: &str| Some((r[part]["line"].as_u64()?, r[part]["character"].as_u64()?));
        let (a, b) = (position("start")?, position("end")?);
        Some(
            a <= (
                u64::from(requested.end_line),
                u64::from(requested.end_character),
            ) && b
                >= (
                    u64::from(requested.start_line),
                    u64::from(requested.start_character),
                ),
        )
    };
    if let Some(diagnostics) = value
        .get("diagnostics")
        .and_then(Value::as_array)
        .filter(|d| !d.is_empty())
    {
        return diagnostics
            .iter()
            .any(|d| overlap(&d["range"]).unwrap_or(true));
    }
    let mut ranges = Vec::new();
    if let Some(changes) = value.pointer("/edit/changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            if crate::protocol::uri_path(uri).as_deref() == Some(path)
                && let Some(edits) = edits.as_array()
            {
                ranges.extend(edits.iter().map(|e| &e["range"]));
            }
        }
    }
    if let Some(changes) = value
        .pointer("/edit/documentChanges")
        .and_then(Value::as_array)
    {
        for change in changes {
            if change
                .pointer("/textDocument/uri")
                .and_then(Value::as_str)
                .and_then(crate::protocol::uri_path)
                .as_deref()
                == Some(path)
                && let Some(edits) = change.get("edits").and_then(Value::as_array)
            {
                ranges.extend(edits.iter().map(|e| &e["range"]));
            }
        }
    }
    ranges.is_empty() || ranges.iter().any(|r| overlap(r).unwrap_or(true))
}
pub(crate) fn list(value: Option<&Value>, resolve: bool) -> Result<(Vec<Action>, bool), String> {
    let empty = vec![];
    let values = match value {
        Some(Value::Null) => &empty,
        Some(Value::Array(v)) => v,
        _ => return Err("CodeAction exige array ou null".into()),
    };
    let truncated = values.len() > 128;
    let mut actions = Vec::new();
    let mut bytes = 0;
    for value in values.iter().take(128) {
        bytes += value.to_string().len();
        if bytes > 1024 * 1024 {
            return Err("CodeActions excedem1MiB normalizado".into());
        }
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
            .ok_or("Título de ação inválido")?
            .to_owned();
        let kind = value
            .get("kind")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .filter(|s| s.len() <= 128 && !s.chars().any(char::is_control))
                    .map(str::to_owned)
                    .ok_or("Kind de ação inválido")
            })
            .transpose()?;
        let mut disabled_reason = match value.get("disabled").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                v.get("reason")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= 1024 && !s.contains('\0'))
                    .ok_or("Motivo de ação desabilitada inválido")?
                    .to_owned(),
            ),
        };
        if value.get("command").is_some_and(|v| !v.is_null()) {
            disabled_reason =
                Some("Esta ação requer comando executável do servidor, não suportado".into());
        } else if value.get("edit").is_none_or(Value::is_null) && !resolve {
            disabled_reason =
                Some("Esta ação não oferece edição e o provider não resolve ações".into());
        }
        let preferred = match value.get("isPreferred") {
            None => false,
            Some(v) => v.as_bool().ok_or("isPreferred inválido")?,
        };
        actions.push(Action {
            item: CodeActionItem {
                token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
                title,
                kind,
                preferred,
                disabled_reason,
            },
            original: value.clone(),
        });
    }
    Ok((actions, truncated))
}
#[cfg(test)]
pub(crate) fn edits(value: &Value, root: &Path) -> Result<Vec<RenameFileEdits>, String> {
    edits_for_language(value, root, crate::Language::Rust)
}
pub(crate) fn edits_for_language(
    value: &Value,
    root: &Path,
    language: crate::Language,
) -> Result<Vec<RenameFileEdits>, String> {
    if value.get("command").is_some_and(|v| !v.is_null())
        || value.get("disabled").is_some_and(|v| !v.is_null())
    {
        return Err("Ação desabilitada ou dependente de comando executável recusada".into());
    }
    let changes = crate::rename::workspace_edit(value.get("edit"), root)?;
    if changes
        .iter()
        .any(|f| crate::Language::for_path(&f.path) != Some(language))
    {
        return Err("Quick fix só pode editar fontes da mesma linguagem dentro do projeto".into());
    }
    Ok(changes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn typescript_auxiliary_command_is_discarded_only_for_complete_edit_without_commands() {
        let safe = json!({"title":"fix","edit":{"changes":{}},"command":{"command":"_typescript.applyCodeActionCommand","arguments":[{"action":{"changes":[],"commands":[]}}]}});
        assert!(edit_only_typescript_action(&safe).get("command").is_none());
        for changed in [
            json!({"title":"fix","command":safe["command"]}),
            json!({"title":"fix","edit":{},"command":{"command":"unknown","arguments":[{"action":{}}]}}),
            json!({"title":"fix","edit":{},"command":{"command":"_typescript.applyCodeActionCommand","arguments":[{"action":{"commands":[{"command":"install"}]}}]}}),
            json!({"title":"fix","edit":{},"command":{"command":"_typescript.applyCodeActionCommand","arguments":[{"action":{"commands":{}}}]}}),
        ] {
            assert!(
                edit_only_typescript_action(&changed)
                    .get("command")
                    .is_some()
            );
        }
    }
    #[test]
    fn commands_disabled_malformed_and_large_actions_never_become_executable() {
        let (items, truncated) = list(
            Some(&json!([
                {"title":"Importar HashMap","kind":"quickfix","data":{"opaque":42}},
                {"title":"Execute","command":"evil"},
                {"title":"Disabled","disabled":{"reason":"Cannot apply"},"edit":{}},
            ])),
            true,
        )
        .unwrap();
        assert!(!truncated);
        assert!(items[0].item.disabled_reason.is_none());
        assert!(items[1].item.disabled_reason.is_some());
        assert_eq!(
            items[2].item.disabled_reason.as_deref(),
            Some("Cannot apply")
        );
        assert!(list(Some(&json!([{"title":"\nControl"}])), true).is_err());
        assert!(list(Some(&json!([{"title":"X","kind":7}])), true).is_err());
        let large = Value::Array((0..140).map(|_| json!({"title":"A"})).collect());
        let (items, truncated) = list(Some(&large), true).unwrap();
        assert_eq!(items.len(), 128);
        assert!(truncated);
    }
    #[test]
    fn workspace_edits_allow_insertions_but_reject_escape_resources_overlap_and_other_languages() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let file = root.join("ação.rs");
        std::fs::write(&file, "// 😀\nfn main(){}\n").unwrap();
        let uri = crate::protocol::file_uri(&file).unwrap();
        let edit = json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"use std::collections::HashMap;\n"});
        let good = json!({"title":"Import","edit":{"changes":{uri.clone():[edit.clone()]}}});
        assert_eq!(edits(&good, &root).unwrap()[0].edits[0].start_character, 0);
        assert!(
            edits(
                &json!({"edit":{"documentChanges":[{"kind":"create","uri":uri}]}}),
                &root
            )
            .is_err()
        );
        assert!(
            edits(
                &json!({"edit":{"changes":{uri:[edit.clone(),edit]}}}),
                &root
            )
            .is_err()
        );
        let other = root.join("x.php");
        std::fs::write(&other, "<?php").unwrap();
        let other = crate::protocol::file_uri(&other).unwrap();
        let bad = json!({"edit":{"changes":{other:[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"x"}]}}});
        assert!(edits(&bad, &root).is_err());
        let outside = tempfile::NamedTempFile::new().unwrap();
        let uri = crate::protocol::file_uri(outside.path()).unwrap();
        let bad = json!({"edit":{"changes":{uri:[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"x"}]}}});
        assert!(edits(&bad, &root).is_err());
    }
}
