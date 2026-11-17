//! Worker-only configuration boundary for moving code files. Configuration is
//! never rewritten by guessing which string is a module reference.
use super::resources::ResourceMove;
use codisseu_syntax::RenameSnapshot;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConfigSnapshot {
    pub path: PathBuf,
    pub contents: Option<Arc<str>>,
}

fn read(path: &Path) -> Result<Option<Arc<str>>, String> {
    let parent = path.parent().ok_or("Configuração sem diretório")?;
    if fs::canonicalize(parent).map_err(|e| e.to_string())? != parent {
        return Err("Diretório de configuração mudou".into());
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 128 * 1024 {
        return Err(format!(
            "Configuração deve ser arquivo regular de até 128 KiB: {}",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 {
        return Err("Configuração excedeu 128 KiB".into());
    }
    Ok(Some(Arc::from(
        String::from_utf8(bytes).map_err(|_| "Configuração não UTF-8")?,
    )))
}

fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

fn touches(value: &str, directory: &Path, moves: &[ResourceMove]) -> bool {
    let value = value.replace('\\', "/");
    let resolved = normalize(&directory.join(&value));
    moves.iter().any(|m| {
        resolved == m.source
            || resolved == m.target
            || [m.source.as_path(), m.target.as_path()].iter().any(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| value.contains(name))
            })
    })
}

fn inspect_strings(value: &Value, directory: &Path, moves: &[ResourceMove]) -> bool {
    match value {
        Value::String(s) => touches(s, directory, moves),
        Value::Array(values) => values.iter().any(|v| inspect_strings(v, directory, moves)),
        Value::Object(values) => values
            .values()
            .any(|v| inspect_strings(v, directory, moves)),
        _ => false,
    }
}

fn present(value: Option<&Value>) -> bool {
    value.is_some_and(|v| match v {
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        _ => true,
    })
}

fn validate(path: &Path, text: &str, moves: &[ResourceMove]) -> Result<(), String> {
    let config: Value = serde_json::from_str(text).map_err(|_| {
        format!(
            "Configuração de movimento exige JSON verificável: {}",
            path.display()
        )
    })?;
    let directory = path.parent().unwrap();
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let error = || {
        format!(
            "Configuração pode depender do nome do arquivo; atualização não comprovada: {}",
            path.display()
        )
    };
    match name {
        "tsconfig.json" | "jsconfig.json" => {
            if present(config.get("extends"))
                || present(config.get("references"))
                || present(config.pointer("/compilerOptions/paths"))
                || present(config.pointer("/compilerOptions/rootDirs"))
            {
                return Err(error());
            }
            for key in ["files", "include", "exclude"] {
                let Some(value) = config.get(key) else {
                    continue;
                };
                let values = value.as_array().ok_or_else(error)?;
                for value in values {
                    let pattern = value.as_str().ok_or_else(error)?;
                    if touches(pattern, directory, moves) {
                        return Err(error());
                    }
                    if key != "files" && pattern.contains(['*', '?', '[', '{']) {
                        let base = pattern.rsplit('/').next().unwrap_or(pattern);
                        // These broad basename patterns retain membership when
                        // only the basename changes and the extension is fixed.
                        if !matches!(
                            base,
                            "*" | "**" | "*.ts" | "*.tsx" | "*.js" | "*.jsx" | "*.php"
                        ) || pattern.contains(['?', '[', '{'])
                        {
                            return Err(error());
                        }
                    }
                }
            }
        }
        "composer.json" => {
            for key in ["autoload", "autoload-dev", "scripts"] {
                if config
                    .get(key)
                    .is_some_and(|v| inspect_strings(v, directory, moves))
                {
                    return Err(error());
                }
            }
        }
        "package.json" => {
            for key in [
                "main", "module", "types", "typings", "browser", "exports", "imports", "bin",
                "scripts",
            ] {
                if config
                    .get(key)
                    .is_some_and(|v| inspect_strings(v, directory, moves))
                {
                    return Err(error());
                }
            }
        }
        _ => return Err("Tipo de configuração desconhecido".into()),
    }
    Ok(())
}

pub(crate) fn capture(
    root: &Path,
    snapshots: &[RenameSnapshot],
    moves: &[ResourceMove],
) -> Result<Vec<ConfigSnapshot>, String> {
    if moves.is_empty() {
        return Ok(vec![]);
    }
    let mut directories = BTreeSet::from([root.to_path_buf()]);
    for path in snapshots
        .iter()
        .map(|s| &s.path)
        .chain(moves.iter().map(|m| &m.source))
    {
        for parent in path.parent().into_iter().flat_map(Path::ancestors) {
            if !parent.starts_with(root) {
                break;
            }
            directories.insert(parent.to_path_buf());
        }
    }
    if directories.len() > 128 {
        return Err("Mais de 128 diretórios de configuração".into());
    }
    let web = moves
        .iter()
        .any(|m| m.source.extension().is_some_and(|e| e != "php"));
    let php = moves
        .iter()
        .any(|m| m.source.extension().is_some_and(|e| e == "php"));
    let mut result = vec![];
    let mut bytes = 0;
    for directory in directories {
        for name in [
            "tsconfig.json",
            "jsconfig.json",
            "package.json",
            "composer.json",
        ] {
            if (name == "composer.json" && !php) || (name != "composer.json" && !web) {
                continue;
            }
            let path = directory.join(name);
            let contents = read(&path)?;
            if let Some(text) = &contents {
                bytes += text.len();
                if bytes > 1024 * 1024 {
                    return Err("Configurações excedem 1 MiB".into());
                }
                validate(&path, text, moves)?;
            }
            result.push(ConfigSnapshot { path, contents });
        }
    }
    Ok(result)
}

pub(crate) fn recheck(snapshots: &[ConfigSnapshot]) -> Result<(), String> {
    for snapshot in snapshots {
        if read(&snapshot.path)? != snapshot.contents {
            return Err(format!(
                "Configuração mudou desde o preview: {}",
                snapshot.path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_file_paths_and_aliases_require_proof_but_broad_globs_survive() {
        let movement = ResourceMove {
            source: "/w/StandaloneTools.ts".into(),
            target: "/w/RenamedTools.ts".into(),
            expected_disk: Arc::from([]),
        };
        let path = Path::new("/w/tsconfig.json");
        for text in [
            r#"{"files":["./StandaloneTools.ts"]}"#,
            r#"{"include":["Stand*.ts"]}"#,
            r#"{"exclude":["**/RenamedTools.ts"]}"#,
            r#"{"compilerOptions":{"paths":{"tools":["./StandaloneTools.ts"]}}}"#,
            r#"{"extends":"../shared.json"}"#,
        ] {
            assert!(
                validate(path, text, std::slice::from_ref(&movement)).is_err(),
                "{text}"
            );
        }
        validate(
            path,
            r#"{"include":["**/*.ts"],"exclude":["node_modules"]}"#,
            std::slice::from_ref(&movement),
        )
        .unwrap();
        assert!(
            validate(
                Path::new("/w/package.json"),
                r#"{"exports":{".":"./StandaloneTools.ts"}}"#,
                std::slice::from_ref(&movement)
            )
            .is_err()
        );
        let php = ResourceMove {
            source: "/w/StandaloneTools.php".into(),
            target: "/w/RenamedTools.php".into(),
            expected_disk: Arc::from([]),
        };
        assert!(
            validate(
                Path::new("/w/composer.json"),
                r#"{"autoload":{"files":["StandaloneTools.php"]}}"#,
                std::slice::from_ref(&php)
            )
            .is_err()
        );
        validate(
            Path::new("/w/composer.json"),
            r#"{"autoload":{"classmap":["src/"]}}"#,
            &[php],
        )
        .unwrap();
    }
    #[test]
    fn absent_created_changed_deleted_and_symlinked_configurations_are_rechecked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let movement = ResourceMove {
            source: root.join("helpers.ts"),
            target: root.join("utilities.ts"),
            expected_disk: Arc::from([]),
        };
        let snapshots = capture(&root, &[], std::slice::from_ref(&movement)).unwrap();
        recheck(&snapshots).unwrap();
        let path = root.join("tsconfig.json");
        fs::write(&path, "{}").unwrap();
        assert!(recheck(&snapshots).is_err());
        let present = capture(&root, &[], std::slice::from_ref(&movement)).unwrap();
        fs::write(&path, "{\n}").unwrap();
        assert!(recheck(&present).is_err());
        fs::remove_file(&path).unwrap();
        assert!(recheck(&present).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("other.json"), &path).unwrap();
            assert!(capture(&root, &[], &[movement]).is_err());
        }
    }
}
