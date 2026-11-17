//! Strict bounded translation of provider navigation metadata. Runs on the LSP
//! worker: URI resolution and metadata access never happen in the GUI thread.
use crate::{DefinitionTarget, protocol};
use serde_json::Value;
use std::path::{Path, PathBuf};

type Position = (u32, u32);
type Range = (Position, Position);

fn position(value: &Value) -> Result<Position, String> {
    let field = |key| {
        value
            .get(key)
            .and_then(Value::as_u64)
            .filter(|n| *n <= i32::MAX as u64)
            .map(|n| n as u32)
            .ok_or_else(|| "Posição de definição inválida".to_string())
    };
    Ok((field("line")?, field("character")?))
}
fn range(value: &Value) -> Result<Range, String> {
    let start = position(value.get("start").ok_or("Range sem início")?)?;
    let end = position(value.get("end").ok_or("Range sem fim")?)?;
    if start > end {
        return Err("Range de definição invertido".into());
    }
    Ok((start, end))
}
pub(crate) fn path(uri: &str, root: &Path) -> Result<PathBuf, String> {
    path_with_roots(uri, root, &[])
}
fn path_with_roots(uri: &str, root: &Path, libraries: &[PathBuf]) -> Result<PathBuf, String> {
    if uri.len() > 4096 || uri.contains(['?', '#', '\0']) {
        return Err("URI de definição inválida ou excessiva".into());
    }
    let local = uri
        .strip_prefix("file://")
        .filter(|s| s.starts_with('/') && !s.starts_with("//"))
        .ok_or("Definição exige URI file local sem autoridade remota")?;
    if local.is_empty() {
        return Err("URI de definição vazia".into());
    }
    let decoded = protocol::uri_path(uri).ok_or("URI de definição inválida")?;
    if decoded.as_os_str().as_encoded_bytes().contains(&0) || !decoded.is_absolute() {
        return Err("Caminho de definição inválido".into());
    }
    let canonical = decoded
        .canonicalize()
        .map_err(|e| format!("Destino de definição indisponível: {e}"))?;
    if !canonical.starts_with(root)
        && !(matches!(
            canonical.extension().and_then(|e| e.to_str()),
            Some("rs" | "py" | "pyi" | "pyw")
        ) && libraries.iter().any(|r| canonical.starts_with(r)))
    {
        return Err("Destino de definição fora do workspace".into());
    }
    if !canonical.is_file() {
        return Err("Destino de definição não é arquivo".into());
    }
    Ok(canonical)
}

pub(crate) fn targets(
    result: Option<&Value>,
    root: &Path,
) -> Result<Vec<DefinitionTarget>, String> {
    targets_with_roots(result, root, &[])
}
pub(crate) fn targets_for_language(
    result: Option<&Value>,
    root: &Path,
    language: crate::Language,
    python: Option<&Path>,
) -> Result<Vec<DefinitionTarget>, String> {
    let roots = if language == crate::Language::Rust {
        crate::rust_libraries::roots()
    } else if language == crate::Language::Python {
        crate::python_libraries::roots(python)
    } else {
        vec![]
    };
    let targets = targets_with_roots(result, root, &roots)?;
    validate_library_language(&targets, root, language)?;
    Ok(targets)
}
fn targets_with_roots(
    result: Option<&Value>,
    root: &Path,
    libraries: &[PathBuf],
) -> Result<Vec<DefinitionTarget>, String> {
    let value = result.ok_or("Resposta de definição sem result")?;
    if value.is_null() {
        return Ok(vec![]);
    }
    let values = if let Some(values) = value.as_array() {
        values.iter().collect::<Vec<_>>()
    } else if value.is_object() {
        vec![value]
    } else {
        return Err("Resposta de definição inválida".into());
    };
    if values.len() > 64 {
        return Err("Definição excede limite de 64 destinos".into());
    }
    let mut result = Vec::new();
    for item in values {
        let target = target(item, root, true, libraries)?;
        if !result.contains(&target) {
            result.push(target);
        }
    }
    Ok(result)
}

fn target(
    item: &Value,
    root: &Path,
    allow_link: bool,
    libraries: &[PathBuf],
) -> Result<DefinitionTarget, String> {
    if !allow_link
        && (item.get("targetUri").is_some()
            || item.get("targetRange").is_some()
            || item.get("targetSelectionRange").is_some())
    {
        return Err("Referências aceitam apenas Location, não LocationLink".into());
    }
    let ((start_line, start_character), (end_line, end_character), uri) =
        if let Some(uri) = item.get("targetUri") {
            let uri = uri.as_str().ok_or("LocationLink sem targetUri válido")?;
            let (outer_start, outer_end) = range(
                item.get("targetRange")
                    .ok_or("LocationLink sem targetRange")?,
            )?;
            let (start, end) = range(
                item.get("targetSelectionRange")
                    .ok_or("LocationLink sem targetSelectionRange")?,
            )?;
            if start < outer_start || end > outer_end {
                return Err("Seleção de definição fora de targetRange".into());
            }
            if let Some(origin) = item.get("originSelectionRange") {
                range(origin)?;
            }
            (start, end, uri)
        } else {
            let uri = item
                .get("uri")
                .and_then(Value::as_str)
                .ok_or("Location sem URI válida")?;
            let (start, end) = range(item.get("range").ok_or("Location sem range")?)?;
            (start, end, uri)
        };
    let target = DefinitionTarget {
        path: path_with_roots(uri, root, libraries)?,
        start_line,
        start_character,
        end_line,
        end_character,
    };
    Ok(target)
}

#[cfg(test)]
pub(crate) fn references(
    result: Option<&Value>,
    root: &Path,
) -> Result<(Vec<DefinitionTarget>, bool), String> {
    references_with_roots(result, root, &[])
}
pub(crate) fn references_for_language(
    result: Option<&Value>,
    root: &Path,
    language: crate::Language,
    python: Option<&Path>,
) -> Result<(Vec<DefinitionTarget>, bool), String> {
    let roots = if language == crate::Language::Rust {
        crate::rust_libraries::roots()
    } else if language == crate::Language::Python {
        crate::python_libraries::roots(python)
    } else {
        vec![]
    };
    let (targets, truncated) = references_with_roots(result, root, &roots)?;
    validate_library_language(&targets, root, language)?;
    Ok((targets, truncated))
}
fn references_with_roots(
    result: Option<&Value>,
    root: &Path,
    libraries: &[PathBuf],
) -> Result<(Vec<DefinitionTarget>, bool), String> {
    let value = result.ok_or("Resposta de referências sem result")?;
    if value.is_null() {
        return Ok((vec![], false));
    }
    let items = value
        .as_array()
        .ok_or("Referências exigem Location[] ou null")?;
    let truncated = items.len() > 256;
    let mut targets = Vec::new();
    for item in items.iter().take(256) {
        let target = target(item, root, false, libraries)?;
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    Ok((targets, truncated))
}

fn validate_library_language(
    targets: &[DefinitionTarget],
    root: &Path,
    language: crate::Language,
) -> Result<(), String> {
    for t in targets.iter().filter(|t| !t.path.starts_with(root)) {
        let ext = t.path.extension().and_then(|e| e.to_str());
        if !match language {
            crate::Language::Rust => ext == Some("rs"),
            crate::Language::Python => matches!(ext, Some("py" | "pyi" | "pyw")),
            _ => false,
        } {
            return Err("Biblioteca de outra linguagem não autorizada".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn span(a: u32, b: u32) -> Value {
        json!({"start":{"line":0,"character":a},"end":{"line":0,"character":b}})
    }
    #[test]
    fn explicit_library_roots_do_not_relax_workspace_rename_or_generic_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let workspace = root.join("workspace");
        let library = root.join("library");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        let source = library.join("lib.rs");
        let other = root.join("private.rs");
        let config = library.join("credentials.toml");
        for p in [&source, &other, &config] {
            std::fs::write(p, "test").unwrap();
        }
        let location = |p: &Path| json!({"uri":protocol::file_uri(p).unwrap(),"range":span(0,4)});
        let value = location(&source);
        assert!(targets(Some(&value), &workspace).is_err());
        assert!(
            path(value["uri"].as_str().unwrap(), &workspace).is_err(),
            "rename still workspace-only"
        );
        assert_eq!(
            targets_with_roots(Some(&value), &workspace, std::slice::from_ref(&library)).unwrap()
                [0]
            .path,
            source
        );
        for p in [&other, &config] {
            assert!(
                targets_with_roots(
                    Some(&location(p)),
                    &workspace,
                    std::slice::from_ref(&library)
                )
                .is_err()
            );
        }
        #[cfg(unix)]
        {
            let link = library.join("escape.rs");
            std::os::unix::fs::symlink(other, &link).unwrap();
            assert!(targets_with_roots(Some(&location(&link)), &workspace, &[library]).is_err());
        }
    }
    #[test]
    fn accepts_location_and_links_selecting_target_selection_and_unicode_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let p = root.join("ação 🌧 #?.ts");
        std::fs::write(&p, "const answer = 42;").unwrap();
        let uri = protocol::file_uri(&p).unwrap();
        let location = json!({"uri":uri,"range":span(6,12)});
        assert_eq!(targets(Some(&location), &root).unwrap()[0].path, p);
        let link =
            json!({"targetUri":uri,"targetRange":span(0,18),"targetSelectionRange":span(6,12)});
        assert_eq!(
            targets(Some(&json!([link])), &root).unwrap()[0].start_character,
            6
        );
        assert_eq!(
            targets(Some(&json!([location.clone(), location])), &root)
                .unwrap()
                .len(),
            1
        );
        assert!(targets(Some(&Value::Null), &root).unwrap().is_empty());
        assert!(targets(Some(&json!([])), &root).unwrap().is_empty());
    }
    #[test]
    fn rejects_escaping_paths_remote_malformed_ranges_and_excessive_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let p = root.join("value.ts");
        std::fs::write(&p, "const x=1;").unwrap();
        let uri = protocol::file_uri(&p).unwrap();
        for target_uri in [
            protocol::file_uri(outside.path()).unwrap(),
            "https://example.invalid/a.ts".into(),
            "file://host/a.ts".into(),
            format!("{uri}?query=1"),
            format!("{uri}#fragment"),
            "file:///%00bad".into(),
            "file:///bad%ZZ".into(),
        ] {
            assert!(targets(Some(&json!({"uri":target_uri,"range":span(0,1)})), &root).is_err());
        }
        assert!(targets(Some(&json!({"uri":uri,"range":span(5,1)})), &root).is_err());
        assert!(targets(Some(&json!({"uri":uri,"range":{"start":{"line":-1,"character":0},"end":{"line":0,"character":1}}})),&root).is_err());
        assert!(targets(Some(&json!({"targetUri":uri,"targetRange":span(0,1),"targetSelectionRange":span(0,2)})),&root).is_err());
        assert!(
            targets(
                Some(&json!(vec![json!({"uri":uri,"range":span(0,1)}); 65])),
                &root
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            let link = root.join("escape.ts");
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            assert!(
                targets(
                    Some(&json!({"uri":protocol::file_uri(&link).unwrap(),"range":span(0,1)})),
                    &root
                )
                .is_err()
            );
        }
    }
    #[test]
    fn references_require_locations_and_report_bounded_prefix_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("ref.ts");
        std::fs::write(&path, "x".repeat(600)).unwrap();
        let uri = protocol::file_uri(&path).unwrap();
        let items = (0..257)
            .map(|i| json!({"uri":uri,"range":span(i,i+1)}))
            .collect::<Vec<_>>();
        let (targets, truncated) = references(Some(&json!(items)), &root).unwrap();
        assert_eq!(targets.len(), 256);
        assert!(truncated);
        assert!(references(Some(&Value::Null), &root).unwrap().0.is_empty());
        assert!(references(Some(&json!({"uri":uri,"range":span(0,1)})), &root).is_err());
        assert!(references(Some(&json!([{"targetUri":uri,"targetRange":span(0,1),"targetSelectionRange":span(0,1)}])),&root).is_err());
    }
}
